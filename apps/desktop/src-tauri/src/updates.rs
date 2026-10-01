use crate::update_download::{
    ArtifactDownloader, BoundedArtifactDownloader, DownloadFailure, PHASE_DOWNLOADING,
    PHASE_VERIFYING, PendingAnnouncement,
};
use crate::update_install::{
    InstallerContext, RetainedTauriUpdate, installation_supported, validate_installer_identity,
};
use nexus_model::{
    AppError, ErrorCode, UpdateAnnouncementId, UpdateOperationSnapshot, UpdatePhase,
    VerifiedArtifactId,
};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::Notify;
use tokio::task::AbortHandle;

const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const CHECK_UNAVAILABLE: &str = "The update service could not be checked right now.";
const DOWNLOAD_UNAVAILABLE: &str = "The update could not be downloaded.";
const VERIFICATION_FAILED: &str = "The downloaded update could not be verified and was discarded.";
const RESOURCE_LIMIT: &str = "The announced update exceeds the supported download size.";
const UPDATE_TIMEOUT: &str = "The update operation timed out.";
const UPDATE_CONFLICT: &str = "The update state changed. Check for updates again.";
const UPDATE_CANCELLED: &str = "The update operation was cancelled.";

pub struct UpdateService {
    inner: Mutex<Inner>,
    downloader: Arc<dyn ArtifactDownloader>,
}

struct Inner {
    generation: u64,
    shutting_down: bool,
    state: State,
}

enum State {
    Idle,
    Checking {
        generation: u64,
    },
    Announced {
        generation: u64,
        id: UpdateAnnouncementId,
        announcement: PendingAnnouncement,
        installer_context: Box<dyn InstallerContext>,
    },
    Downloading {
        generation: u64,
        id: UpdateAnnouncementId,
        version: String,
        phase: Arc<AtomicU8>,
        abort: AbortHandle,
        completion: Arc<TaskCompletion>,
        installer_context: Box<dyn InstallerContext>,
    },
    Verified {
        generation: u64,
        verified_artifact_id: Option<VerifiedArtifactId>,
        version: String,
        installer_context: Box<dyn InstallerContext>,
        bytes: Vec<u8>,
    },
    Installing {
        generation: u64,
        version: String,
    },
}

struct CheckedAnnouncement {
    announcement: PendingAnnouncement,
    installer_context: Box<dyn InstallerContext>,
}

pub(crate) struct InstallAuthority {
    installer_context: Box<dyn InstallerContext>,
    bytes: Vec<u8>,
}

impl InstallAuthority {
    pub(crate) fn new(installer_context: Box<dyn InstallerContext>, bytes: Vec<u8>) -> Self {
        Self {
            installer_context,
            bytes,
        }
    }

    pub(crate) fn invoke(self) -> Result<(), AppError> {
        self.installer_context.invoke(self.bytes)
    }
}

type DownloadTask = tokio::task::JoinHandle<Result<Vec<u8>, DownloadFailure>>;
type BegunDownload = (u64, String, DownloadTask);

struct TaskCompletion {
    done: AtomicBool,
    notify: Notify,
}

impl TaskCompletion {
    fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    async fn wait(&self) {
        if self.done.load(Ordering::Acquire) {
            return;
        }
        let notified = self.notify.notified();
        if self.done.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

struct TaskCompletionGuard(Arc<TaskCompletion>);

impl Drop for TaskCompletionGuard {
    fn drop(&mut self) {
        self.0.done.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }
}

impl Default for UpdateService {
    fn default() -> Self {
        Self::with_downloader(Arc::new(BoundedArtifactDownloader))
    }
}

impl UpdateService {
    fn with_downloader(downloader: Arc<dyn ArtifactDownloader>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                generation: 0,
                shutting_down: false,
                state: State::Idle,
            }),
            downloader,
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, AppError> {
        self.inner
            .lock()
            .map_err(|_| AppError::new(ErrorCode::UpdateConflict, UPDATE_CONFLICT))
    }

    pub fn snapshot(&self, current_version: String) -> Result<UpdateOperationSnapshot, AppError> {
        let inner = self.lock()?;
        Ok(snapshot_for(&inner.state, current_version))
    }

    pub async fn check(&self, app: &AppHandle) -> Result<UpdateOperationSnapshot, AppError> {
        let current_version = app.package_info().version.to_string();
        let generation = self.begin_check()?;
        let result = async {
            let updater = app
                .updater_builder()
                .timeout(CHECK_TIMEOUT)
                .no_proxy()
                .restart_after_install(false)
                .on_before_exit(|| {})
                .build()
                .map_err(|_| check_unavailable())?;
            let announced = updater.check().await.map_err(|_| check_unavailable())?;
            announced
                .map(|update| {
                    let announcement = PendingAnnouncement::new(
                        update.version.clone(),
                        update.download_url.clone(),
                        update.signature.clone(),
                    )
                    .map_err(|_| check_unavailable())?;
                    Ok::<CheckedAnnouncement, AppError>(CheckedAnnouncement {
                        announcement,
                        installer_context: Box::new(RetainedTauriUpdate::new(update)),
                    })
                })
                .transpose()
        }
        .await;

        match result {
            Ok(Some(checked)) => self.finish_announced(generation, current_version, checked),
            Ok(None) => self.finish_up_to_date(generation, current_version),
            Err(error) => {
                self.fail_active(generation)?;
                Err(error)
            }
        }
    }

    pub async fn download(
        &self,
        current_version: String,
        announcement_id: UpdateAnnouncementId,
    ) -> Result<UpdateOperationSnapshot, AppError> {
        let (generation, version, task) = self.begin_download(announcement_id)?;
        let result = match task.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Err(DownloadFailure::Cancelled),
            Err(_) => Err(DownloadFailure::Download),
        };

        let mut inner = self.lock()?;
        if inner.shutting_down {
            return Err(cancelled());
        }
        let previous = std::mem::replace(&mut inner.state, State::Idle);
        let installer_context = match previous {
            State::Downloading {
                generation: active_generation,
                id,
                installer_context,
                ..
            } if active_generation == generation && id == announcement_id => installer_context,
            other => {
                inner.state = other;
                return Err(cancelled());
            }
        };

        match result {
            Ok(bytes) => {
                let verified_artifact_id = if installation_supported() {
                    if validate_installer_identity(&installer_context.identity()).is_err() {
                        return Err(download_error(DownloadFailure::Verification));
                    }
                    Some(VerifiedArtifactId::new())
                } else {
                    None
                };
                inner.state = State::Verified {
                    generation,
                    verified_artifact_id,
                    version,
                    installer_context,
                    bytes,
                };
                Ok(snapshot_for(&inner.state, current_version))
            }
            Err(error) => {
                inner.state = State::Idle;
                Err(download_error(error))
            }
        }
    }

    pub async fn shutdown(&self) -> Result<(), AppError> {
        let active = {
            let mut inner = self.lock()?;
            inner.shutting_down = true;
            match &inner.state {
                State::Downloading {
                    abort, completion, ..
                } => {
                    abort.abort();
                    Some(Arc::clone(completion))
                }
                _ => {
                    inner.state = State::Idle;
                    None
                }
            }
        };
        if let Some(completion) = active {
            tokio::time::timeout(SHUTDOWN_TIMEOUT, completion.wait())
                .await
                .map_err(|_| AppError::new(ErrorCode::UpdateTimeout, UPDATE_TIMEOUT))?;
            self.lock()?.state = State::Idle;
        }
        Ok(())
    }

    fn begin_check(&self) -> Result<u64, AppError> {
        let mut inner = self.lock()?;
        if inner.shutting_down {
            return Err(cancelled());
        }
        if matches!(
            inner.state,
            State::Checking { .. } | State::Downloading { .. } | State::Installing { .. }
        ) {
            return Err(conflict());
        }
        let generation = inner.generation.checked_add(1).ok_or_else(conflict)?;
        inner.generation = generation;
        inner.state = State::Checking { generation };
        Ok(generation)
    }

    fn finish_announced(
        &self,
        generation: u64,
        current_version: String,
        checked: CheckedAnnouncement,
    ) -> Result<UpdateOperationSnapshot, AppError> {
        let mut inner = self.lock()?;
        if inner.shutting_down
            || !matches!(inner.state, State::Checking { generation: active } if active == generation)
        {
            return Err(cancelled());
        }
        let id = UpdateAnnouncementId::new();
        inner.state = State::Announced {
            generation,
            id,
            announcement: checked.announcement,
            installer_context: checked.installer_context,
        };
        Ok(snapshot_for(&inner.state, current_version))
    }

    fn finish_up_to_date(
        &self,
        generation: u64,
        current_version: String,
    ) -> Result<UpdateOperationSnapshot, AppError> {
        let mut inner = self.lock()?;
        if inner.shutting_down
            || !matches!(inner.state, State::Checking { generation: active } if active == generation)
        {
            return Err(cancelled());
        }
        inner.state = State::Idle;
        Ok(UpdateOperationSnapshot {
            current_version,
            phase: UpdatePhase::UpToDate,
            available_version: None,
            announcement_id: None,
            verified_artifact_id: None,
            installation_supported: false,
        })
    }

    fn fail_active(&self, generation: u64) -> Result<(), AppError> {
        let mut inner = self.lock()?;
        if matches!(inner.state, State::Checking { generation: active } if active == generation) {
            inner.state = State::Idle;
        }
        Ok(())
    }

    fn begin_download(
        &self,
        announcement_id: UpdateAnnouncementId,
    ) -> Result<BegunDownload, AppError> {
        let mut inner = self.lock()?;
        if inner.shutting_down {
            return Err(cancelled());
        }
        let previous = std::mem::replace(&mut inner.state, State::Idle);
        let (generation, id, announcement, installer_context) = match previous {
            State::Announced {
                generation,
                id,
                announcement,
                installer_context,
            } if id == announcement_id => (generation, id, announcement, installer_context),
            other => {
                inner.state = other;
                return Err(conflict());
            }
        };
        let version = announcement.version.clone();
        let phase = Arc::new(AtomicU8::new(PHASE_DOWNLOADING));
        let completion = Arc::new(TaskCompletion::new());
        let guard = TaskCompletionGuard(Arc::clone(&completion));
        let downloader = Arc::clone(&self.downloader);
        let task_phase = Arc::clone(&phase);
        let task = tokio::spawn(async move {
            let _completion = guard;
            downloader.download(announcement, task_phase).await
        });
        inner.state = State::Downloading {
            generation,
            id,
            version: version.clone(),
            phase,
            abort: task.abort_handle(),
            completion,
            installer_context,
        };
        Ok((generation, version, task))
    }

    #[cfg(test)]
    fn seed_announcement(
        &self,
        generation: u64,
        announcement: PendingAnnouncement,
        installer_context: Box<dyn InstallerContext>,
    ) -> UpdateAnnouncementId {
        let id = UpdateAnnouncementId::new();
        let mut inner = self.inner.lock().unwrap();
        inner.generation = generation;
        inner.state = State::Announced {
            generation,
            id,
            announcement,
            installer_context,
        };
        id
    }

    pub(crate) fn validate_install(
        &self,
        verified_artifact_id: VerifiedArtifactId,
    ) -> Result<(), AppError> {
        if !installation_supported() {
            return Err(AppError::new(
                ErrorCode::UpdateInstall,
                "In-app update installation is not available on this platform.",
            ));
        }
        let inner = self.lock()?;
        match &inner.state {
            State::Verified {
                verified_artifact_id: Some(active_id),
                installer_context,
                ..
            } if *active_id == verified_artifact_id => {
                validate_installer_identity(&installer_context.identity())
            }
            _ => Err(conflict()),
        }
    }

    pub(crate) fn consume_install(
        &self,
        verified_artifact_id: VerifiedArtifactId,
    ) -> Result<InstallAuthority, AppError> {
        let mut inner = self.lock()?;
        let previous = std::mem::replace(&mut inner.state, State::Idle);
        match previous {
            State::Verified {
                generation,
                verified_artifact_id: Some(active_id),
                version,
                installer_context,
                bytes,
            } if active_id == verified_artifact_id => {
                validate_installer_identity(&installer_context.identity())?;
                inner.state = State::Installing {
                    generation,
                    version,
                };
                Ok(InstallAuthority::new(installer_context, bytes))
            }
            other => {
                inner.state = other;
                Err(install_conflict())
            }
        }
    }
}

fn snapshot_for(state: &State, current_version: String) -> UpdateOperationSnapshot {
    let (phase, available_version, announcement_id, verified_artifact_id) = match state {
        State::Idle => (UpdatePhase::Idle, None, None, None),
        State::Checking { .. } => (UpdatePhase::Checking, None, None, None),
        State::Announced {
            id, announcement, ..
        } => (
            UpdatePhase::UpdateAnnounced,
            Some(announcement.version.clone()),
            Some(*id),
            None,
        ),
        State::Downloading { version, phase, .. } => {
            let phase = match phase.load(Ordering::Acquire) {
                PHASE_VERIFYING => UpdatePhase::Verifying,
                _ => UpdatePhase::Downloading,
            };
            (phase, Some(version.clone()), None, None)
        }
        State::Verified {
            generation,
            verified_artifact_id,
            version,
            ..
        } => {
            let _ = generation;
            (
                UpdatePhase::Verified,
                Some(version.clone()),
                None,
                *verified_artifact_id,
            )
        }
        State::Installing {
            generation,
            version,
        } => {
            let _ = generation;
            (UpdatePhase::Installing, Some(version.clone()), None, None)
        }
    };
    UpdateOperationSnapshot {
        current_version,
        phase,
        available_version,
        announcement_id,
        verified_artifact_id,
        installation_supported: installation_supported() && verified_artifact_id.is_some(),
    }
}

fn check_unavailable() -> AppError {
    AppError::new(ErrorCode::UpdateCheck, CHECK_UNAVAILABLE)
}

fn conflict() -> AppError {
    AppError::new(ErrorCode::UpdateConflict, UPDATE_CONFLICT)
}

fn install_conflict() -> AppError {
    AppError::new(
        ErrorCode::UpdateConflict,
        "The verified update authority changed. Close and restart NexusOps before continuing.",
    )
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, UPDATE_CANCELLED)
}

fn download_error(error: DownloadFailure) -> AppError {
    match error {
        DownloadFailure::Download => AppError::new(ErrorCode::UpdateDownload, DOWNLOAD_UNAVAILABLE),
        DownloadFailure::Verification => {
            AppError::new(ErrorCode::UpdateVerification, VERIFICATION_FAILED)
        }
        DownloadFailure::ResourceLimit => {
            AppError::new(ErrorCode::UpdateResourceLimit, RESOURCE_LIMIT)
        }
        DownloadFailure::Timeout => AppError::new(ErrorCode::UpdateTimeout, UPDATE_TIMEOUT),
        DownloadFailure::Cancelled => cancelled(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update_install::InstallerIdentity;
    use reqwest::Url;
    use std::future;
    use std::sync::atomic::AtomicBool as TestAtomicBool;

    struct FakeInstaller {
        identity: InstallerIdentity,
        invocations: Option<Arc<Mutex<Vec<Vec<u8>>>>>,
    }

    impl InstallerContext for FakeInstaller {
        fn identity(&self) -> InstallerIdentity {
            self.identity.clone()
        }

        fn invoke(self: Box<Self>, bytes: Vec<u8>) -> Result<(), AppError> {
            if let Some(invocations) = &self.invocations {
                invocations.lock().unwrap().push(bytes);
            }
            Ok(())
        }
    }

    struct DropObservedInstaller {
        identity: InstallerIdentity,
        dropped: Arc<TestAtomicBool>,
    }

    impl Drop for DropObservedInstaller {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    impl InstallerContext for DropObservedInstaller {
        fn identity(&self) -> InstallerIdentity {
            self.identity.clone()
        }

        fn invoke(self: Box<Self>, _bytes: Vec<u8>) -> Result<(), AppError> {
            Ok(())
        }
    }

    struct FixedDownloader(Result<Vec<u8>, DownloadFailure>);

    impl ArtifactDownloader for FixedDownloader {
        fn download<'a>(
            &'a self,
            _announcement: PendingAnnouncement,
            phase: Arc<AtomicU8>,
        ) -> crate::update_download::DownloadFuture<'a> {
            Box::pin(async move {
                phase.store(PHASE_VERIFYING, Ordering::Release);
                self.0.clone()
            })
        }
    }

    struct BlockingDownloader {
        started: Arc<Notify>,
    }

    impl ArtifactDownloader for BlockingDownloader {
        fn download<'a>(
            &'a self,
            _announcement: PendingAnnouncement,
            _phase: Arc<AtomicU8>,
        ) -> crate::update_download::DownloadFuture<'a> {
            Box::pin(async move {
                self.started.notify_one();
                future::pending::<Result<Vec<u8>, DownloadFailure>>().await
            })
        }
    }

    fn announcement(version: &str) -> PendingAnnouncement {
        #[cfg(target_arch = "x86_64")]
        let architecture = "x64";
        #[cfg(target_arch = "aarch64")]
        let architecture = "arm64";
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let architecture = "unsupported";
        PendingAnnouncement {
            version: version.into(),
            url: Url::parse(&format!(
                "https://github.com/unrealbg/NexusOps/releases/download/v1/NexusOps_{version}_{architecture}-setup.exe"
            ))
            .unwrap(),
            signature: "test-only".into(),
        }
    }

    fn installer(version: &str) -> Box<dyn InstallerContext> {
        let announcement = announcement(version);
        Box::new(FakeInstaller {
            identity: InstallerIdentity {
                version: version.into(),
                target: "windows".into(),
                artifact_basename: announcement
                    .url
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .map(ToOwned::to_owned),
            },
            invocations: None,
        })
    }

    fn seed(service: &UpdateService, generation: u64, version: &str) -> UpdateAnnouncementId {
        service.seed_announcement(generation, announcement(version), installer(version))
    }

    #[tokio::test]
    async fn exact_announcement_is_consumed_once_and_verified_bytes_remain_native() {
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![1, 2, 3]))));
        let id = seed(&service, 1, "0.2.0");
        let snapshot = service.download("0.1.0".into(), id).await.unwrap();
        assert_eq!(snapshot.phase, UpdatePhase::Verified);
        assert_eq!(snapshot.available_version.as_deref(), Some("0.2.0"));
        assert_eq!(snapshot.announcement_id, None);
        assert_eq!(
            snapshot.verified_artifact_id.is_some(),
            installation_supported()
        );
        assert_eq!(
            service.download("0.1.0".into(), id).await.unwrap_err().code,
            ErrorCode::UpdateConflict
        );
    }

    #[tokio::test]
    async fn stale_id_and_network_interruption_require_a_fresh_check() {
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Err(
            DownloadFailure::Download,
        ))));
        let id = seed(&service, 1, "0.2.0");
        assert_eq!(
            service
                .download("0.1.0".into(), UpdateAnnouncementId::new())
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpdateConflict
        );
        assert_eq!(
            service.download("0.1.0".into(), id).await.unwrap_err().code,
            ErrorCode::UpdateDownload
        );
        assert_eq!(
            service.download("0.1.0".into(), id).await.unwrap_err().code,
            ErrorCode::UpdateConflict
        );
        assert_eq!(
            service.snapshot("0.1.0".into()).unwrap().phase,
            UpdatePhase::Idle
        );
    }

    #[tokio::test]
    async fn active_download_rejects_second_download_and_check() {
        let started = Arc::new(Notify::new());
        let service = Arc::new(UpdateService::with_downloader(Arc::new(
            BlockingDownloader {
                started: Arc::clone(&started),
            },
        )));
        let id = seed(&service, 1, "0.2.0");
        let running = {
            let service = Arc::clone(&service);
            tokio::spawn(async move { service.download("0.1.0".into(), id).await })
        };
        started.notified().await;
        assert_eq!(
            service.download("0.1.0".into(), id).await.unwrap_err().code,
            ErrorCode::UpdateConflict
        );
        assert_eq!(
            service.begin_check().unwrap_err().code,
            ErrorCode::UpdateConflict
        );
        service.shutdown().await.unwrap();
        assert_eq!(
            running.await.unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
    }

    #[test]
    fn new_check_invalidates_announced_and_verified_authority() {
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![]))));
        let old = seed(&service, 2, "0.2.0");
        let next = service.begin_check().unwrap();
        assert_eq!(next, 3);
        assert!(matches!(
            service.inner.lock().unwrap().state,
            State::Checking { generation: 3 }
        ));
        assert_ne!(old, UpdateAnnouncementId::new());

        {
            let mut inner = service.inner.lock().unwrap();
            inner.state = State::Verified {
                generation: 3,
                verified_artifact_id: None,
                version: "0.2.0".into(),
                installer_context: installer("0.2.0"),
                bytes: vec![1],
            };
        }
        assert_eq!(service.begin_check().unwrap(), 4);
        assert!(matches!(
            service.inner.lock().unwrap().state,
            State::Checking { generation: 4 }
        ));
    }

    #[test]
    fn generation_overflow_fails_closed_without_rebinding() {
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![]))));
        service.inner.lock().unwrap().generation = u64::MAX;
        assert_eq!(
            service.begin_check().unwrap_err().code,
            ErrorCode::UpdateConflict
        );
        assert!(matches!(service.inner.lock().unwrap().state, State::Idle));
    }

    #[tokio::test]
    async fn shutdown_aborts_a_blocked_body_read_and_clears_authority() {
        let started = Arc::new(Notify::new());
        let service = Arc::new(UpdateService::with_downloader(Arc::new(
            BlockingDownloader {
                started: Arc::clone(&started),
            },
        )));
        let id = seed(&service, 1, "0.2.0");
        let running = {
            let service = Arc::clone(&service);
            tokio::spawn(async move { service.download("0.1.0".into(), id).await })
        };
        started.notified().await;
        service.shutdown().await.unwrap();
        assert_eq!(
            running.await.unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert_eq!(
            service.snapshot("0.1.0".into()).unwrap().phase,
            UpdatePhase::Idle
        );
        service.shutdown().await.unwrap();
        assert_eq!(
            service.snapshot("0.1.0".into()).unwrap().phase,
            UpdatePhase::Idle
        );
        assert_eq!(
            service.begin_check().unwrap_err().code,
            ErrorCode::Cancelled
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn verified_install_authority_is_exact_and_one_shot() {
        let invocations = Arc::new(Mutex::new(Vec::new()));
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![4, 5, 6]))));
        let context = FakeInstaller {
            identity: installer("0.2.0").identity(),
            invocations: Some(Arc::clone(&invocations)),
        };
        let announcement_id =
            service.seed_announcement(7, announcement("0.2.0"), Box::new(context));
        let snapshot = service
            .download("0.1.0".into(), announcement_id)
            .await
            .unwrap();
        let verified_id = snapshot.verified_artifact_id.unwrap();
        service.validate_install(verified_id).unwrap();
        assert_eq!(
            service
                .validate_install(VerifiedArtifactId::new())
                .unwrap_err()
                .code,
            ErrorCode::UpdateConflict
        );
        service
            .consume_install(verified_id)
            .unwrap()
            .invoke()
            .unwrap();
        assert_eq!(*invocations.lock().unwrap(), [vec![4, 5, 6]]);
        assert_eq!(
            service.consume_install(verified_id).err().unwrap().code,
            ErrorCode::UpdateConflict
        );
        assert_eq!(
            service.validate_install(verified_id).unwrap_err().code,
            ErrorCode::UpdateConflict
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn invalid_windows_installer_identity_never_grants_authority() {
        for basename in [
            "NexusOps_0.2.0_arm64-setup.exe",
            "NexusOps_0.2.0_x64.msi",
            "Other_0.2.0_x64-setup.exe",
        ] {
            let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![1]))));
            let id = service.seed_announcement(
                1,
                announcement("0.2.0"),
                Box::new(FakeInstaller {
                    identity: InstallerIdentity {
                        version: "0.2.0".into(),
                        target: "windows".into(),
                        artifact_basename: Some(basename.into()),
                    },
                    invocations: None,
                }),
            );
            assert_eq!(
                service.download("0.1.0".into(), id).await.unwrap_err().code,
                ErrorCode::UpdateVerification
            );
            assert_eq!(
                service.snapshot("0.1.0".into()).unwrap().phase,
                UpdatePhase::Idle
            );
        }
    }

    #[tokio::test]
    async fn failed_download_drops_the_retained_installer_context() {
        let dropped = Arc::new(TestAtomicBool::new(false));
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Err(
            DownloadFailure::Download,
        ))));
        let id = service.seed_announcement(
            1,
            announcement("0.2.0"),
            Box::new(DropObservedInstaller {
                identity: installer("0.2.0").identity(),
                dropped: Arc::clone(&dropped),
            }),
        );
        assert!(service.download("0.1.0".into(), id).await.is_err());
        assert!(dropped.load(Ordering::Acquire));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn new_check_and_shutdown_drop_verified_install_authority() {
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![8]))));
        let id = seed(&service, 1, "0.2.0");
        let verified = service
            .download("0.1.0".into(), id)
            .await
            .unwrap()
            .verified_artifact_id
            .unwrap();
        assert_eq!(service.begin_check().unwrap(), 2);
        assert_eq!(
            service.validate_install(verified).unwrap_err().code,
            ErrorCode::UpdateConflict
        );

        let second = seed(&service, 3, "0.3.0");
        let verified = service
            .download("0.1.0".into(), second)
            .await
            .unwrap()
            .verified_artifact_id
            .unwrap();
        service.shutdown().await.unwrap();
        assert_eq!(
            service.validate_install(verified).unwrap_err().code,
            ErrorCode::UpdateConflict
        );
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn unsupported_platform_never_mints_install_authority() {
        let service = UpdateService::with_downloader(Arc::new(FixedDownloader(Ok(vec![1]))));
        let id = seed(&service, 1, "0.2.0");
        let snapshot = service.download("0.1.0".into(), id).await.unwrap();
        assert_eq!(snapshot.verified_artifact_id, None);
        assert!(!snapshot.installation_supported);
        assert_eq!(
            service
                .validate_install(VerifiedArtifactId::new())
                .unwrap_err()
                .code,
            ErrorCode::UpdateInstall
        );
    }
}
