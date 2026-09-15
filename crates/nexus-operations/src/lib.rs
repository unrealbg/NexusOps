//! Read-only policy boundary between domain providers and remote transports.
//! No API in this crate accepts a caller-supplied shell command.

mod command;
mod engine;

pub use command::ReadOnlyCommand;
pub use engine::{OperationEngine, OperationPlan, RollbackStatus};

use async_trait::async_trait;
use nexus_model::AppError;
use tokio_util::sync::CancellationToken;

/// A connection owns its transport state; unrelated hosts never share a session.
/// Implementations must bound command time/output and stop work on cancellation.
#[async_trait]
pub trait RemoteSession: Send + Sync {
    async fn execute(
        &self,
        command: ReadOnlyCommand,
        cancellation: CancellationToken,
    ) -> Result<String, AppError>;

    async fn disconnect(&self) -> Result<(), AppError>;

    fn is_closed(&self) -> bool;
}
