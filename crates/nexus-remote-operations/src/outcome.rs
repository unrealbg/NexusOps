use crate::{ConsumedAuthority, NativeOperation};
use async_trait::async_trait;
use nexus_model::AppError;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteOperationOutcome {
    Success,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotDispatchedReason {
    Cancelled,
    Timeout,
    Connection,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionUnknownReason {
    Cancelled,
    Timeout,
    ConnectionLost,
    OutputLimit,
}

/// Transport result encodes whether mutation dispatch could have happened.
/// Raw stdout and stderr have no representation here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationTransportOutcome {
    NotDispatched(NotDispatchedReason),
    CompletionConfirmed { success: bool },
    CompletionUnknown(CompletionUnknownReason),
}

impl MutationTransportOutcome {
    pub fn operation_outcome(self) -> RemoteOperationOutcome {
        match self {
            Self::NotDispatched(NotDispatchedReason::Cancelled) => {
                RemoteOperationOutcome::Cancelled
            }
            Self::NotDispatched(_) => RemoteOperationOutcome::Failed,
            Self::CompletionConfirmed { success: true } => RemoteOperationOutcome::Success,
            Self::CompletionConfirmed { success: false } => RemoteOperationOutcome::Failed,
            Self::CompletionUnknown(_) => RemoteOperationOutcome::OutcomeUnknown,
        }
    }
}

#[async_trait]
pub trait AuthorityRevalidator<O: NativeOperation>: Send + Sync {
    type DispatchGuard: Send;

    async fn revalidate(
        &self,
        authority: &ConsumedAuthority<O>,
    ) -> Result<Self::DispatchGuard, AppError>;
}

#[async_trait]
pub trait MutationTransport<O: NativeOperation>: Send + Sync {
    async fn dispatch(
        &self,
        authority: &ConsumedAuthority<O>,
        cancellation: CancellationToken,
    ) -> MutationTransportOutcome;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionResult {
    pub outcome: RemoteOperationOutcome,
    pub terminal: ExecutionTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionTerminal {
    RevalidationFailed,
    CancelledBeforeDispatch,
    Transport(MutationTransportOutcome),
}
