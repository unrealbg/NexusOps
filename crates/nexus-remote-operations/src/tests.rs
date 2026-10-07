use super::*;
use async_trait::async_trait;
use nexus_model::{AppError, ErrorCode, HostId, HostSessionId, OperationRisk};
use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

struct FakeOperation;
impl super::authority::sealed::Sealed for FakeOperation {}
impl NativeOperation for FakeOperation {
    type Target = u64;
    type Preconditions = u64;
    type Payload = [u8; 8];

    const RISK: OperationRisk = OperationRisk::Moderate;
}

struct OtherOperation;
impl super::authority::sealed::Sealed for OtherOperation {}
impl NativeOperation for OtherOperation {
    type Target = u64;
    type Preconditions = u64;
    type Payload = ();

    const RISK: OperationRisk = OperationRisk::High;
}

fn binding(host: HostId) -> AuthorityBinding {
    AuthorityBinding {
        host_id: host,
        host_session_id: HostSessionId::new(),
        generation: 7,
    }
}

fn draft(binding: AuthorityBinding, value: u64) -> PlanDraft<FakeOperation> {
    PlanDraft::new(binding, value, value + 1, value.to_le_bytes())
}

fn insert(store: &AuthorityStore, binding: AuthorityBinding, value: u64) -> RemoteOperationPlanId {
    store.insert(draft(binding, value), binding).unwrap().id
}

#[tokio::test(start_paused = true)]
async fn opaque_ids_ttl_and_supersession_are_bounded_per_host() {
    let store = AuthorityStore::with_ttl(PLAN_TTL);
    let first_host = binding(HostId::new());
    let second_host = binding(HostId::new());
    let first = store.insert(draft(first_host, 1), first_host).unwrap();
    let replacement = store.insert(draft(first_host, 2), first_host).unwrap();
    let other = store.insert(draft(second_host, 3), second_host).unwrap();

    assert_ne!(first.id, replacement.id);
    assert_ne!(replacement.id, other.id);
    assert_eq!(replacement.superseded, Some(first.id));
    assert!(!store.contains(first.id).unwrap());
    assert!(store.contains(replacement.id).unwrap());
    assert!(store.contains(other.id).unwrap());
    assert_eq!(replacement.risk, OperationRisk::Moderate);
    assert_eq!(replacement.expires_at - replacement.issued_at, PLAN_TTL);
    assert_eq!(store.pending_count().unwrap(), 2);

    tokio::time::advance(PLAN_TTL).await;
    tokio::task::yield_now().await;
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn insertion_rejects_a_delayed_stale_planning_result() {
    let store = AuthorityStore::default();
    let planned = binding(HostId::new());
    let current = AuthorityBinding {
        host_session_id: HostSessionId::new(),
        generation: planned.generation + 1,
        ..planned
    };
    assert_eq!(
        store.insert(draft(planned, 1), current).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn exact_binding_operation_and_risk_are_required_before_consume() {
    let store = AuthorityStore::default();
    let owner = binding(HostId::new());
    let id = insert(&store, owner, 9);
    for wrong in [
        AuthorityBinding {
            host_id: HostId::new(),
            ..owner
        },
        AuthorityBinding {
            host_session_id: HostSessionId::new(),
            ..owner
        },
        AuthorityBinding {
            generation: owner.generation + 1,
            ..owner
        },
    ] {
        assert_eq!(
            store
                .consume::<FakeOperation>(id, wrong)
                .err()
                .unwrap()
                .code,
            ErrorCode::Conflict
        );
        assert!(store.contains(id).unwrap());
    }
    assert_eq!(
        store
            .consume::<OtherOperation>(id, owner)
            .err()
            .unwrap()
            .code,
        ErrorCode::Policy
    );
    assert!(store.contains(id).unwrap());
    store.corrupt_risk(id, OperationRisk::Low);
    assert_eq!(
        store
            .consume::<FakeOperation>(id, owner)
            .err()
            .unwrap()
            .code,
        ErrorCode::Policy
    );
}

#[test]
fn consume_is_one_shot_and_moves_the_private_native_payload() {
    let store = AuthorityStore::default();
    let owner = binding(HostId::new());
    let id = insert(&store, owner, 42);
    let authority = store.consume::<FakeOperation>(id, owner).unwrap();
    assert_eq!(authority.id(), id);
    assert_eq!(authority.binding(), owner);
    assert_eq!(authority.risk(), OperationRisk::Moderate);
    assert_eq!(*authority.target(), 42);
    assert_eq!(*authority.preconditions(), 43);
    assert_eq!(*authority.payload(), 42_u64.to_le_bytes());
    assert_eq!(
        store
            .consume::<FakeOperation>(id, owner)
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn discard_is_exact_idempotent_and_scoped_to_the_authority_owner() {
    let store = AuthorityStore::default();
    let owner = binding(HostId::new());
    let id = insert(&store, owner, 1);
    let wrong = AuthorityBinding {
        host_session_id: HostSessionId::new(),
        ..owner
    };
    assert_eq!(
        store.discard::<FakeOperation>(id, wrong).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(store.contains(id).unwrap());
    assert!(store.discard::<FakeOperation>(id, owner).unwrap());
    assert!(!store.discard::<FakeOperation>(id, owner).unwrap());
}

#[test]
fn consume_and_discard_share_one_race_owner() {
    for _ in 0..64 {
        let store = Arc::new(AuthorityStore::default());
        let owner = binding(HostId::new());
        let id = insert(&store, owner, 1);
        let barrier = Arc::new(Barrier::new(3));
        let consuming = {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.consume::<FakeOperation>(id, owner).is_ok()
            })
        };
        let discarding = {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.discard::<FakeOperation>(id, owner).unwrap_or(false)
            })
        };
        barrier.wait();
        assert_ne!(consuming.join().unwrap(), discarding.join().unwrap());
        assert!(!store.contains(id).unwrap());
    }
}

#[tokio::test(start_paused = true)]
async fn consume_and_expiry_share_one_race_owner() {
    let store = AuthorityStore::with_ttl(Duration::from_secs(1));
    let owner = binding(HostId::new());
    let id = insert(&store, owner, 1);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        store
            .consume::<FakeOperation>(id, owner)
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    tokio::task::yield_now().await;
    assert!(!store.contains(id).unwrap());
}

#[test]
fn two_simultaneous_consumers_dispatch_at_most_one_authority() {
    let store = Arc::new(AuthorityStore::default());
    let owner = binding(HostId::new());
    let id = insert(&store, owner, 1);
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            store.consume::<FakeOperation>(id, owner).is_ok()
        }));
    }
    barrier.wait();
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|won| *won)
            .count(),
        1
    );
}

#[test]
fn turnover_edit_delete_and_shutdown_revocation_are_terminal() {
    let store = AuthorityStore::default();
    let first = binding(HostId::new());
    let second = binding(HostId::new());
    let first_id = insert(&store, first, 1);
    let second_id = insert(&store, second, 2);
    assert!(
        store
            .revoke_session(first.host_id, first.host_session_id)
            .unwrap()
    );
    assert!(!store.contains(first_id).unwrap());
    assert!(store.revoke_host(second.host_id).unwrap());
    assert!(!store.contains(second_id).unwrap());

    let _ = insert(&store, first, 3);
    let _ = insert(&store, second, 4);
    store.revoke_all().unwrap();
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn execution_admission_is_non_queuing_per_host_and_global() {
    let admission = ExecutionAdmission::default();
    let first_host = HostId::new();
    let second_host = HostId::new();
    let first = admission.try_execute(first_host).unwrap();
    assert_eq!(
        admission.try_execute(first_host).err().unwrap().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        admission.try_execute(second_host).err().unwrap().code,
        ErrorCode::Conflict
    );
    drop(first);
    assert!(admission.try_execute(second_host).is_ok());
}

struct FakeRevalidator {
    allow: AtomicBool,
    entered: Notify,
    release: Notify,
    stall: AtomicBool,
}

impl FakeRevalidator {
    fn allowed() -> Self {
        Self {
            allow: AtomicBool::new(true),
            entered: Notify::new(),
            release: Notify::new(),
            stall: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl AuthorityRevalidator<FakeOperation> for FakeRevalidator {
    type DispatchGuard = ();

    async fn revalidate(&self, _: &ConsumedAuthority<FakeOperation>) -> Result<(), AppError> {
        self.entered.notify_one();
        if self.stall.load(Ordering::SeqCst) {
            self.release.notified().await;
        }
        if self.allow.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::Conflict,
                "The backend precondition changed.",
            ))
        }
    }
}

struct FakeTransport {
    outcome: MutationTransportOutcome,
    calls: AtomicUsize,
}

struct RejectingRevalidator(&'static str);

#[async_trait]
impl AuthorityRevalidator<FakeOperation> for RejectingRevalidator {
    type DispatchGuard = ();

    async fn revalidate(&self, _: &ConsumedAuthority<FakeOperation>) -> Result<(), AppError> {
        Err(AppError::new(ErrorCode::Conflict, self.0))
    }
}

#[async_trait]
impl MutationTransport<FakeOperation> for FakeTransport {
    async fn dispatch(
        &self,
        _: &ConsumedAuthority<FakeOperation>,
        _: tokio_util::sync::CancellationToken,
    ) -> MutationTransportOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.outcome
    }
}

fn transport(outcome: MutationTransportOutcome) -> FakeTransport {
    FakeTransport {
        outcome,
        calls: AtomicUsize::new(0),
    }
}

#[tokio::test]
async fn capacity_failure_before_consume_preserves_authority_and_dispatches_nothing() {
    let foundation = RemoteOperationFoundation::default();
    let owner = binding(HostId::new());
    let id = foundation
        .authorities
        .insert(draft(owner, 1), owner)
        .unwrap()
        .id;
    let _held = foundation.admission.try_execute(owner.host_id).unwrap();
    let transport = transport(MutationTransportOutcome::CompletionConfirmed { success: true });
    let error = foundation
        .execute::<FakeOperation, _, _>(
            id,
            owner,
            &FakeRevalidator::allowed(),
            &transport,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(foundation.authorities.contains(id).unwrap());
}

#[tokio::test]
async fn failed_revalidation_dispatches_nothing_and_consumed_authority_never_returns() {
    let foundation = RemoteOperationFoundation::default();
    let owner = binding(HostId::new());
    let id = foundation
        .authorities
        .insert(draft(owner, 1), owner)
        .unwrap()
        .id;
    let revalidator = FakeRevalidator::allowed();
    revalidator.allow.store(false, Ordering::SeqCst);
    let transport = transport(MutationTransportOutcome::CompletionConfirmed { success: true });
    let result = foundation
        .execute::<FakeOperation, _, _>(
            id,
            owner,
            &revalidator,
            &transport,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.outcome, RemoteOperationOutcome::Failed);
    assert_eq!(result.terminal, ExecutionTerminal::RevalidationFailed);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(!foundation.authorities.contains(id).unwrap());
}

#[tokio::test]
async fn cancellation_during_revalidation_stays_cancelled_and_dispatches_nothing() {
    let foundation = Arc::new(RemoteOperationFoundation::default());
    let owner = binding(HostId::new());
    let id = foundation
        .authorities
        .insert(draft(owner, 1), owner)
        .unwrap()
        .id;
    let revalidator = Arc::new(FakeRevalidator::allowed());
    revalidator.stall.store(true, Ordering::SeqCst);
    let transport = Arc::new(transport(MutationTransportOutcome::CompletionConfirmed {
        success: true,
    }));
    let cancellation = tokio_util::sync::CancellationToken::new();
    let execution = {
        let foundation = Arc::clone(&foundation);
        let revalidator = Arc::clone(&revalidator);
        let transport = Arc::clone(&transport);
        let cancellation = cancellation.clone();
        tokio::spawn(async move {
            foundation
                .execute::<FakeOperation, _, _>(
                    id,
                    owner,
                    revalidator.as_ref(),
                    transport.as_ref(),
                    cancellation,
                )
                .await
                .unwrap()
        })
    };
    revalidator.entered.notified().await;
    cancellation.cancel();
    revalidator.release.notify_one();
    let result = execution.await.unwrap();
    assert_eq!(result.outcome, RemoteOperationOutcome::Cancelled);
    assert_eq!(result.terminal, ExecutionTerminal::CancelledBeforeDispatch);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(!foundation.authorities.contains(id).unwrap());
}

#[tokio::test]
async fn every_backend_revalidation_failure_dispatches_nothing() {
    for reason in [
        "host removed",
        "session replaced",
        "generation changed",
        "transport closed",
        "target precondition changed",
        "operation no longer permitted",
    ] {
        let foundation = RemoteOperationFoundation::default();
        let owner = binding(HostId::new());
        let id = foundation.plan(draft(owner, 1), owner).unwrap().id;
        let transport = transport(MutationTransportOutcome::CompletionConfirmed { success: true });
        let result = foundation
            .execute::<FakeOperation, _, _>(
                id,
                owner,
                &RejectingRevalidator(reason),
                &transport,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.outcome, RemoteOperationOutcome::Failed, "{reason}");
        assert_eq!(
            result.terminal,
            ExecutionTerminal::RevalidationFailed,
            "{reason}"
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 0, "{reason}");
        assert!(!foundation.authorities.contains(id).unwrap(), "{reason}");
    }
}

#[tokio::test(start_paused = true)]
async fn expired_authority_never_revalidates_or_dispatches() {
    let foundation = RemoteOperationFoundation {
        authorities: AuthorityStore::with_ttl(Duration::from_secs(1)),
        admission: ExecutionAdmission::default(),
    };
    let owner = binding(HostId::new());
    let id = foundation.plan(draft(owner, 1), owner).unwrap().id;
    tokio::time::advance(Duration::from_secs(1)).await;
    let transport = transport(MutationTransportOutcome::CompletionConfirmed { success: true });
    assert_eq!(
        foundation
            .execute::<FakeOperation, _, _>(
                id,
                owner,
                &FakeRevalidator::allowed(),
                &transport,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fake_transport_covers_all_dispatch_certainty_outcomes_without_retry() {
    let cases = [
        (
            MutationTransportOutcome::CompletionConfirmed { success: true },
            RemoteOperationOutcome::Success,
        ),
        (
            MutationTransportOutcome::CompletionConfirmed { success: false },
            RemoteOperationOutcome::Failed,
        ),
        (
            MutationTransportOutcome::NotDispatched(NotDispatchedReason::Cancelled),
            RemoteOperationOutcome::Cancelled,
        ),
        (
            MutationTransportOutcome::NotDispatched(NotDispatchedReason::Timeout),
            RemoteOperationOutcome::Failed,
        ),
        (
            MutationTransportOutcome::NotDispatched(NotDispatchedReason::Connection),
            RemoteOperationOutcome::Failed,
        ),
        (
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::Timeout),
            RemoteOperationOutcome::OutcomeUnknown,
        ),
        (
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::ConnectionLost),
            RemoteOperationOutcome::OutcomeUnknown,
        ),
        (
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::Cancelled),
            RemoteOperationOutcome::OutcomeUnknown,
        ),
        (
            MutationTransportOutcome::CompletionUnknown(CompletionUnknownReason::OutputLimit),
            RemoteOperationOutcome::OutcomeUnknown,
        ),
    ];
    for (transport_outcome, expected) in cases {
        let foundation = RemoteOperationFoundation::default();
        let owner = binding(HostId::new());
        let id = foundation
            .authorities
            .insert(draft(owner, 1), owner)
            .unwrap()
            .id;
        let transport = transport(transport_outcome);
        let result = foundation
            .execute::<FakeOperation, _, _>(
                id,
                owner,
                &FakeRevalidator::allowed(),
                &transport,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.outcome, expected);
        assert_eq!(
            result.terminal,
            ExecutionTerminal::Transport(transport_outcome)
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        assert!(!foundation.authorities.contains(id).unwrap());
    }
}

#[tokio::test]
async fn cancellation_before_dispatch_is_cancelled_and_never_calls_transport() {
    let foundation = RemoteOperationFoundation::default();
    let owner = binding(HostId::new());
    let id = foundation
        .authorities
        .insert(draft(owner, 1), owner)
        .unwrap()
        .id;
    let cancellation = tokio_util::sync::CancellationToken::new();
    cancellation.cancel();
    let transport = transport(MutationTransportOutcome::CompletionUnknown(
        CompletionUnknownReason::Cancelled,
    ));
    let result = foundation
        .execute::<FakeOperation, _, _>(
            id,
            owner,
            &FakeRevalidator::allowed(),
            &transport,
            cancellation,
        )
        .await
        .unwrap();
    assert_eq!(result.outcome, RemoteOperationOutcome::Cancelled);
    assert_eq!(result.terminal, ExecutionTerminal::CancelledBeforeDispatch);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(!foundation.authorities.contains(id).unwrap());
}

#[tokio::test]
async fn shutdown_rejects_new_execution_revokes_pending_and_never_dispatches() {
    let foundation = RemoteOperationFoundation::default();
    let owner = binding(HostId::new());
    let id = foundation
        .authorities
        .insert(draft(owner, 1), owner)
        .unwrap()
        .id;
    foundation.begin_shutdown().unwrap();
    let transport = transport(MutationTransportOutcome::CompletionConfirmed { success: true });
    assert!(
        foundation
            .execute::<FakeOperation, _, _>(
                id,
                owner,
                &FakeRevalidator::allowed(),
                &transport,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .is_err()
    );
    assert_eq!(foundation.authorities.pending_count().unwrap(), 0);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        foundation.plan(draft(owner, 2), owner).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn a_long_host_operation_does_not_own_unrelated_state_locks() {
    let admission = Arc::new(ExecutionAdmission::default());
    let first = HostId::new();
    let second = HostId::new();
    let permit = admission.try_execute(first).unwrap();
    let unrelated_state = Arc::new(tokio::sync::Mutex::new(()));
    let acquired = Arc::new(AtomicBool::new(false));
    let task = {
        let state = Arc::clone(&unrelated_state);
        let acquired = Arc::clone(&acquired);
        tokio::spawn(async move {
            let _guard = state.lock().await;
            acquired.store(true, Ordering::SeqCst);
        })
    };
    task.await.unwrap();
    assert!(acquired.load(Ordering::SeqCst));
    assert_eq!(permit.host_id, first);
    assert_eq!(
        admission.try_execute(second).err().unwrap().code,
        ErrorCode::Conflict
    );
}
