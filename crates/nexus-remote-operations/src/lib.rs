//! Internal one-shot authority foundation for future reviewed remote mutations.
//!
//! Goal 05A intentionally exposes no production operation, shell command, IPC
//! model, SSH mutation mapping, automatic retry, or generic rollback.

mod admission;
mod authority;
mod outcome;
mod store;
mod systemd_reset_failed;
mod systemd_try_restart;

pub use admission::{ExecutionAdmission, ExecutionPermit};
pub use authority::{AuthorityBinding, ConsumedAuthority, NativeOperation, PlanDraft, PlanReceipt};
pub use nexus_model::RemoteOperationPlanId;
pub use outcome::{
    AuthorityRevalidator, CompletionUnknownReason, ExecutionResult, ExecutionTerminal,
    MutationTransport, MutationTransportOutcome, NotDispatchedReason, RemoteOperationOutcome,
};
pub use store::{AuthorityStore, PLAN_TTL};
pub use systemd_reset_failed::{
    SystemdResetFailed, SystemdResetFailedPreconditions, SystemdServiceUnitName,
};
pub use systemd_try_restart::{SystemdTryRestart, SystemdTryRestartPreconditions};

use nexus_model::{AppError, ErrorCode, HostId, HostSessionId};
use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct RemoteOperationFoundation {
    authorities: AuthorityStore,
    admission: ExecutionAdmission,
}

impl RemoteOperationFoundation {
    pub fn plan<O: NativeOperation>(
        &self,
        draft: PlanDraft<O>,
        current: AuthorityBinding,
    ) -> Result<PlanReceipt, AppError> {
        self.authorities.insert(draft, current)
    }

    pub fn discard<O: NativeOperation>(
        &self,
        plan_id: RemoteOperationPlanId,
        binding: AuthorityBinding,
    ) -> Result<bool, AppError> {
        self.authorities.discard::<O>(plan_id, binding)
    }

    pub async fn lifecycle_guard(&self, host_id: HostId) -> Result<OwnedMutexGuard<()>, AppError> {
        self.admission.lifecycle_guard(host_id).await
    }

    pub fn try_lifecycle_guard(&self, host_id: HostId) -> Result<OwnedMutexGuard<()>, AppError> {
        self.admission.try_lifecycle_guard(host_id)
    }

    pub fn revoke_host(&self, host_id: HostId) -> Result<bool, AppError> {
        self.authorities.revoke_host(host_id)
    }

    pub fn revoke_session(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<bool, AppError> {
        self.authorities.revoke_session(host_id, host_session_id)
    }

    pub fn begin_shutdown(&self) -> Result<(), AppError> {
        self.authorities.seal_and_revoke_all()
    }

    pub async fn execute<O, R, T>(
        &self,
        plan_id: RemoteOperationPlanId,
        binding: AuthorityBinding,
        revalidator: &R,
        transport: &T,
        cancellation: CancellationToken,
    ) -> Result<ExecutionResult, AppError>
    where
        O: NativeOperation,
        R: AuthorityRevalidator<O>,
        T: MutationTransport<O> + ?Sized,
    {
        if self.authorities.is_sealed()? {
            return Err(shutting_down());
        }
        // Non-queuing admission precedes consumption, so a busy result preserves
        // pending authority and never schedules hidden background execution.
        let _permit = self.admission.try_execute(binding.host_id)?;
        if self.authorities.is_sealed()? {
            return Err(shutting_down());
        }
        let authority = self.authorities.consume::<O>(plan_id, binding)?;
        if cancellation.is_cancelled() {
            return Ok(ExecutionResult {
                outcome: RemoteOperationOutcome::Cancelled,
                terminal: ExecutionTerminal::CancelledBeforeDispatch,
            });
        }
        let dispatch_guard = match revalidator.revalidate(&authority).await {
            Ok(guard) => guard,
            Err(_) => {
                if cancellation.is_cancelled() {
                    return Ok(ExecutionResult {
                        outcome: RemoteOperationOutcome::Cancelled,
                        terminal: ExecutionTerminal::CancelledBeforeDispatch,
                    });
                }
                return Ok(ExecutionResult {
                    outcome: RemoteOperationOutcome::Failed,
                    terminal: ExecutionTerminal::RevalidationFailed,
                });
            }
        };
        if self.authorities.is_sealed().unwrap_or(true) || cancellation.is_cancelled() {
            return Ok(ExecutionResult {
                outcome: RemoteOperationOutcome::Cancelled,
                terminal: ExecutionTerminal::CancelledBeforeDispatch,
            });
        }
        let transport_outcome = transport.dispatch(&authority, cancellation).await;
        drop(dispatch_guard);
        Ok(ExecutionResult {
            outcome: transport_outcome.operation_outcome(),
            terminal: ExecutionTerminal::Transport(transport_outcome),
        })
    }
}

fn shutting_down() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "NexusOps is shutting down; create a fresh plan after restart.",
    )
}

#[cfg(test)]
mod tests;
