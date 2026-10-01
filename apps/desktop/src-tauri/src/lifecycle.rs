use nexus_core::Application;
use nexus_model::{AppError, ErrorCode};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;
use tracing_appender::non_blocking::WorkerGuard;

const COMMAND_DRAIN_TIMEOUT: Duration = Duration::from_secs(60);
const CLOSING_MESSAGE: &str = "NexusOps is closing. Restart the application to continue.";
const DRAIN_TIMEOUT_MESSAGE: &str =
    "NexusOps is still closing. Wait for active operations and try closing again.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifecyclePhase {
    Running,
    Sealed,
    Exclusive,
    Quiesced,
}

struct LifecycleState {
    phase: LifecyclePhase,
    active: u64,
}

struct SharedLifecycle {
    state: Mutex<LifecycleState>,
    changed: Notify,
}

pub struct LifecycleCoordinator {
    shared: Arc<SharedLifecycle>,
}

pub struct CommandPermit {
    shared: Arc<SharedLifecycle>,
}

impl CommandPermit {
    /// Irreversibly wins installation ownership while retaining this permit.
    /// The caller is drained only against every other admitted command.
    pub async fn seal_and_drain_others(&self) -> Result<(), AppError> {
        self.seal_and_drain_others_for(COMMAND_DRAIN_TIMEOUT).await
    }

    async fn seal_and_drain_others_for(&self, timeout: Duration) -> Result<(), AppError> {
        {
            let mut state = self.shared.state.lock().map_err(|_| closing_error())?;
            if state.phase != LifecyclePhase::Running {
                return Err(closing_error());
            }
            state.phase = LifecyclePhase::Exclusive;
            self.shared.changed.notify_waiters();
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.shared.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.shared.state.lock().map_err(|_| closing_error())?;
                if state.active == 1 {
                    return Ok(());
                }
            }
            tokio::time::timeout_at(deadline, notified)
                .await
                .map_err(|_| AppError::new(ErrorCode::Timeout, DRAIN_TIMEOUT_MESSAGE))?;
        }
    }
}

impl Default for LifecycleCoordinator {
    fn default() -> Self {
        Self {
            shared: Arc::new(SharedLifecycle {
                state: Mutex::new(LifecycleState {
                    phase: LifecyclePhase::Running,
                    active: 0,
                }),
                changed: Notify::new(),
            }),
        }
    }
}

impl LifecycleCoordinator {
    pub fn admit(&self) -> Result<CommandPermit, AppError> {
        let mut state = self.shared.state.lock().map_err(|_| closing_error())?;
        if state.phase != LifecyclePhase::Running {
            return Err(closing_error());
        }
        let Some(active) = state.active.checked_add(1) else {
            state.phase = LifecyclePhase::Sealed;
            self.shared.changed.notify_waiters();
            return Err(closing_error());
        };
        state.active = active;
        Ok(CommandPermit {
            shared: Arc::clone(&self.shared),
        })
    }

    pub async fn seal_and_drain(&self) -> Result<(), AppError> {
        self.seal_and_drain_for(COMMAND_DRAIN_TIMEOUT).await
    }

    async fn seal_and_drain_for(&self, timeout: Duration) -> Result<(), AppError> {
        self.seal()?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.shared.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.shared.state.lock().map_err(|_| closing_error())?;
                if state.active == 0 {
                    state.phase = LifecyclePhase::Quiesced;
                    return Ok(());
                }
            }
            tokio::time::timeout_at(deadline, notified)
                .await
                .map_err(|_| AppError::new(ErrorCode::Timeout, DRAIN_TIMEOUT_MESSAGE))?;
        }
    }

    fn seal(&self) -> Result<(), AppError> {
        let mut state = self.shared.state.lock().map_err(|_| closing_error())?;
        if state.phase == LifecyclePhase::Running {
            state.phase = LifecyclePhase::Sealed;
        }
        Ok(())
    }
}

impl Drop for CommandPermit {
    fn drop(&mut self) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        let Some(active) = state.active.checked_sub(1) else {
            state.phase = LifecyclePhase::Sealed;
            self.shared.changed.notify_waiters();
            return;
        };
        state.active = active;
        self.shared.changed.notify_waiters();
    }
}

fn closing_error() -> AppError {
    AppError::new(ErrorCode::Conflict, CLOSING_MESSAGE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitRequest {
    StartCleanup,
    PreventExit,
    AllowExit,
}

#[derive(Default)]
struct ExitState {
    attempt_running: bool,
    cleanup_complete: bool,
}

#[derive(Default)]
pub struct ExitCoordinator {
    state: Mutex<ExitState>,
}

impl ExitCoordinator {
    pub fn request(&self) -> ExitRequest {
        let Ok(mut state) = self.state.lock() else {
            return ExitRequest::PreventExit;
        };
        if state.cleanup_complete {
            ExitRequest::AllowExit
        } else if state.attempt_running {
            ExitRequest::PreventExit
        } else {
            state.attempt_running = true;
            ExitRequest::StartCleanup
        }
    }

    pub fn complete(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.attempt_running = false;
            state.cleanup_complete = true;
        }
    }

    pub fn failed(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.attempt_running = false;
        }
    }
}

struct GuardSlot<T> {
    value: Mutex<Option<T>>,
}

impl<T> GuardSlot<T> {
    fn new(value: T) -> Self {
        Self {
            value: Mutex::new(Some(value)),
        }
    }

    pub(crate) fn finalize(&self) -> Result<(), AppError> {
        let value = self
            .value
            .lock()
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Persistence,
                    "NexusOps could not finish closing its local log.",
                )
            })?
            .take();
        drop(value);
        Ok(())
    }
}

pub struct LogGuardService {
    guard: GuardSlot<WorkerGuard>,
}

impl LogGuardService {
    pub fn new(guard: WorkerGuard) -> Self {
        Self {
            guard: GuardSlot::new(guard),
        }
    }

    pub(crate) fn finalize(&self) -> Result<(), AppError> {
        self.guard.finalize()
    }
}

#[derive(Debug)]
pub struct ExitCleanupFailure {
    pub stage: &'static str,
    pub error: AppError,
}

trait ExitCleanupSteps {
    fn seal_and_drain(&self) -> impl Future<Output = Result<(), AppError>> + Send;
    fn shutdown_updates(&self) -> impl Future<Output = Result<(), AppError>> + Send;
    fn shutdown_application(&self) -> impl Future<Output = Result<(), AppError>> + Send;
    fn revoke_local_grants(&self) -> Result<(), AppError>;
    fn finalize_logging(&self) -> Result<(), AppError>;
}

struct TauriExitCleanup {
    app: AppHandle,
}

impl ExitCleanupSteps for TauriExitCleanup {
    async fn seal_and_drain(&self) -> Result<(), AppError> {
        self.app
            .state::<LifecycleCoordinator>()
            .seal_and_drain()
            .await
    }

    async fn shutdown_updates(&self) -> Result<(), AppError> {
        self.app
            .state::<crate::updates::UpdateService>()
            .shutdown()
            .await
    }

    async fn shutdown_application(&self) -> Result<(), AppError> {
        self.app.state::<Application>().shutdown().await
    }

    fn revoke_local_grants(&self) -> Result<(), AppError> {
        self.app
            .state::<crate::local_access::LocalAccessService>()
            .revoke_all()
    }

    fn finalize_logging(&self) -> Result<(), AppError> {
        self.app.state::<LogGuardService>().finalize()
    }
}

async fn run_exit_cleanup(steps: &impl ExitCleanupSteps) -> Result<(), ExitCleanupFailure> {
    steps
        .seal_and_drain()
        .await
        .map_err(|error| ExitCleanupFailure {
            stage: "command_drain",
            error,
        })?;
    steps
        .shutdown_updates()
        .await
        .map_err(|error| ExitCleanupFailure {
            stage: "update_shutdown",
            error,
        })?;
    steps
        .shutdown_application()
        .await
        .map_err(|error| ExitCleanupFailure {
            stage: "application_shutdown",
            error,
        })?;
    steps
        .revoke_local_grants()
        .map_err(|error| ExitCleanupFailure {
            stage: "local_grant_revocation",
            error,
        })?;
    steps
        .finalize_logging()
        .map_err(|error| ExitCleanupFailure {
            stage: "logging_finalize",
            error,
        })?;
    Ok(())
}

async fn run_exit_attempt(
    coordinator: &ExitCoordinator,
    steps: &impl ExitCleanupSteps,
    request_final_exit: impl FnOnce(),
) -> Result<(), ExitCleanupFailure> {
    match run_exit_cleanup(steps).await {
        Ok(()) => {
            coordinator.complete();
            request_final_exit();
            Ok(())
        }
        Err(failure) => {
            coordinator.failed();
            Err(failure)
        }
    }
}

pub async fn run_tauri_exit_attempt(app: AppHandle) -> Result<(), ExitCleanupFailure> {
    let exit_app = app.clone();
    let cleanup_app = app.clone();
    run_exit_attempt(
        &app.state::<ExitCoordinator>(),
        &TauriExitCleanup { app: cleanup_app },
        move || exit_app.exit(0),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn admission_is_serialized_counted_and_seals_on_overflow() {
        let coordinator = LifecycleCoordinator::default();
        let permit = coordinator.admit().unwrap();
        assert_eq!(coordinator.shared.state.lock().unwrap().active, 1);
        drop(permit);
        assert_eq!(coordinator.shared.state.lock().unwrap().active, 0);

        coordinator.shared.state.lock().unwrap().active = u64::MAX;
        let error = coordinator.admit().err().unwrap();
        assert_eq!(error, closing_error());
        assert_eq!(
            coordinator.shared.state.lock().unwrap().phase,
            LifecyclePhase::Sealed
        );
    }

    #[tokio::test]
    async fn seal_with_no_active_work_is_immediate_and_irreversible() {
        let coordinator = LifecycleCoordinator::default();
        coordinator.seal_and_drain().await.unwrap();
        coordinator.seal_and_drain().await.unwrap();
        assert_eq!(
            coordinator.shared.state.lock().unwrap().phase,
            LifecyclePhase::Quiesced
        );
        assert_eq!(coordinator.admit().err().unwrap(), closing_error());
    }

    #[test]
    fn admission_seal_race_is_either_counted_or_rejected() {
        for _ in 0..128 {
            let coordinator = Arc::new(LifecycleCoordinator::default());
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let admitting = {
                let coordinator = Arc::clone(&coordinator);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    coordinator.admit()
                })
            };
            let sealing = {
                let coordinator = Arc::clone(&coordinator);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    coordinator.seal()
                })
            };
            barrier.wait();
            let permit = admitting.join().unwrap();
            sealing.join().unwrap().unwrap();
            let state = coordinator.shared.state.lock().unwrap();
            assert_ne!(state.phase, LifecyclePhase::Running);
            match permit {
                Ok(permit) => {
                    assert_eq!(state.active, 1);
                    drop(state);
                    drop(permit);
                }
                Err(error) => {
                    assert_eq!(error, closing_error());
                    assert_eq!(state.active, 0);
                }
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drain_waits_for_the_last_permit_and_rejects_new_work() {
        let coordinator = Arc::new(LifecycleCoordinator::default());
        let first = coordinator.admit().unwrap();
        let second = coordinator.admit().unwrap();
        let draining = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.seal_and_drain().await })
        };
        tokio::task::yield_now().await;
        assert_eq!(coordinator.admit().err().unwrap(), closing_error());
        drop(first);
        tokio::task::yield_now().await;
        assert!(!draining.is_finished());
        drop(second);
        draining.await.unwrap().unwrap();
        assert_eq!(
            coordinator.shared.state.lock().unwrap().phase,
            LifecyclePhase::Quiesced
        );
        coordinator.seal_and_drain().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn exclusive_drain_waits_for_other_permits_but_not_its_caller() {
        let coordinator = Arc::new(LifecycleCoordinator::default());
        let installer = coordinator.admit().unwrap();
        let other = coordinator.admit().unwrap();
        let draining = tokio::spawn(async move {
            installer.seal_and_drain_others().await?;
            Ok::<CommandPermit, AppError>(installer)
        });
        tokio::task::yield_now().await;
        assert!(!draining.is_finished());
        assert_eq!(coordinator.admit().err().unwrap(), closing_error());
        drop(other);
        let installer = draining.await.unwrap().unwrap();
        assert_eq!(
            coordinator.shared.state.lock().unwrap().phase,
            LifecyclePhase::Exclusive
        );
        assert_eq!(coordinator.shared.state.lock().unwrap().active, 1);
        drop(installer);
        coordinator.seal_and_drain().await.unwrap();
        assert_eq!(
            coordinator.shared.state.lock().unwrap().phase,
            LifecyclePhase::Quiesced
        );
    }

    #[tokio::test(start_paused = true)]
    async fn normal_exit_and_install_exclusive_have_deterministic_ownership() {
        let exit_wins = LifecycleCoordinator::default();
        let install_permit = exit_wins.admit().unwrap();
        exit_wins.seal().unwrap();
        assert_eq!(
            install_permit.seal_and_drain_others().await.unwrap_err(),
            closing_error()
        );
        drop(install_permit);
        exit_wins.seal_and_drain().await.unwrap();

        let install_wins = Arc::new(LifecycleCoordinator::default());
        let install_permit = install_wins.admit().unwrap();
        install_permit.seal_and_drain_others().await.unwrap();
        let closing = {
            let coordinator = Arc::clone(&install_wins);
            tokio::spawn(async move { coordinator.seal_and_drain().await })
        };
        tokio::task::yield_now().await;
        assert!(!closing.is_finished());
        drop(install_permit);
        closing.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn exclusive_timeout_stays_sealed_until_permit_drop_allows_exit() {
        let coordinator = Arc::new(LifecycleCoordinator::default());
        let installer = coordinator.admit().unwrap();
        let other = coordinator.admit().unwrap();
        let draining = tokio::spawn(async move {
            let result = installer
                .seal_and_drain_others_for(COMMAND_DRAIN_TIMEOUT)
                .await;
            (result, installer)
        });
        tokio::task::yield_now().await;
        tokio::time::advance(COMMAND_DRAIN_TIMEOUT).await;
        let (result, installer) = draining.await.unwrap();
        assert_eq!(result.unwrap_err().code, ErrorCode::Timeout);
        assert_eq!(coordinator.admit().err().unwrap(), closing_error());
        drop(other);
        drop(installer);
        coordinator.seal_and_drain().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn drain_timeout_stays_sealed_and_a_later_retry_can_finish() {
        let coordinator = Arc::new(LifecycleCoordinator::default());
        let permit = coordinator.admit().unwrap();
        let draining = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.seal_and_drain().await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(COMMAND_DRAIN_TIMEOUT).await;
        assert_eq!(
            draining.await.unwrap().unwrap_err().code,
            ErrorCode::Timeout
        );
        assert_eq!(coordinator.admit().err().unwrap(), closing_error());
        drop(permit);
        coordinator
            .seal_and_drain_for(Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[test]
    fn exit_requests_start_once_retry_after_failure_and_allow_only_after_success() {
        let coordinator = ExitCoordinator::default();
        assert_eq!(coordinator.request(), ExitRequest::StartCleanup);
        assert_eq!(coordinator.request(), ExitRequest::PreventExit);
        coordinator.failed();
        assert_eq!(coordinator.request(), ExitRequest::StartCleanup);
        coordinator.complete();
        assert_eq!(coordinator.request(), ExitRequest::AllowExit);
        assert_eq!(coordinator.request(), ExitRequest::AllowExit);
    }

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn logging_guard_is_consumed_exactly_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = GuardSlot::new(DropCounter(Arc::clone(&drops)));
        slot.finalize().unwrap();
        slot.finalize().unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    struct RecordedSteps {
        calls: Mutex<Vec<&'static str>>,
        fail_at: Option<&'static str>,
    }

    impl RecordedSteps {
        fn step(&self, name: &'static str) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(name);
            if self.fail_at == Some(name) {
                Err(AppError::new(ErrorCode::Conflict, "safe test failure"))
            } else {
                Ok(())
            }
        }
    }

    impl ExitCleanupSteps for RecordedSteps {
        async fn seal_and_drain(&self) -> Result<(), AppError> {
            self.step("command_drain")
        }
        async fn shutdown_updates(&self) -> Result<(), AppError> {
            self.step("update_shutdown")
        }
        async fn shutdown_application(&self) -> Result<(), AppError> {
            self.step("application_shutdown")
        }
        fn revoke_local_grants(&self) -> Result<(), AppError> {
            self.step("local_grant_revocation")
        }
        fn finalize_logging(&self) -> Result<(), AppError> {
            self.step("logging_finalize")
        }
    }

    #[tokio::test]
    async fn cleanup_order_is_fixed_and_stops_at_the_first_failure() {
        let success = RecordedSteps {
            calls: Mutex::new(vec![]),
            fail_at: None,
        };
        run_exit_cleanup(&success).await.unwrap();
        assert_eq!(
            *success.calls.lock().unwrap(),
            [
                "command_drain",
                "update_shutdown",
                "application_shutdown",
                "local_grant_revocation",
                "logging_finalize"
            ]
        );

        let failure = RecordedSteps {
            calls: Mutex::new(vec![]),
            fail_at: Some("application_shutdown"),
        };
        let error = run_exit_cleanup(&failure).await.unwrap_err();
        assert_eq!(error.stage, "application_shutdown");
        assert_eq!(
            *failure.calls.lock().unwrap(),
            ["command_drain", "update_shutdown", "application_shutdown"]
        );
    }

    #[tokio::test]
    async fn final_exit_is_requested_only_after_success_and_failure_can_be_retried() {
        let coordinator = ExitCoordinator::default();
        assert_eq!(coordinator.request(), ExitRequest::StartCleanup);
        let exit_requests = AtomicUsize::new(0);
        let failed_drain = RecordedSteps {
            calls: Mutex::new(vec![]),
            fail_at: Some("command_drain"),
        };
        run_exit_attempt(&coordinator, &failed_drain, || {
            exit_requests.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap_err();
        assert_eq!(*failed_drain.calls.lock().unwrap(), ["command_drain"]);
        assert_eq!(exit_requests.load(Ordering::SeqCst), 0);
        assert_eq!(coordinator.request(), ExitRequest::StartCleanup);

        let failed = RecordedSteps {
            calls: Mutex::new(vec![]),
            fail_at: Some("application_shutdown"),
        };
        run_exit_attempt(&coordinator, &failed, || {
            exit_requests.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap_err();
        assert_eq!(exit_requests.load(Ordering::SeqCst), 0);
        assert_eq!(coordinator.request(), ExitRequest::StartCleanup);

        let success = RecordedSteps {
            calls: Mutex::new(vec![]),
            fail_at: None,
        };
        run_exit_attempt(&coordinator, &success, || {
            exit_requests.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();
        assert_eq!(exit_requests.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.request(), ExitRequest::AllowExit);
    }
}
