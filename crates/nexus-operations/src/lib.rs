//! Read-only policy boundary between domain providers and remote transports.
//! No API in this crate accepts a caller-supplied shell command.

mod command;
mod engine;
mod stop_impact;

pub use command::ReadOnlyCommand;
pub use engine::{OperationEngine, OperationPlan, RollbackStatus};
pub use stop_impact::{
    SYSTEMD_STOP_IMPACT_PROPERTIES, SystemdStopImpactEngine, SystemdStopImpactQuery,
};

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

    /// Executes the single reviewed, dynamically targeted read-only systemd
    /// inspection command. Implementations must not expose a generic string
    /// execution surface; the query type validates and constructs the command.
    async fn execute_stop_impact(
        &self,
        _query: &SystemdStopImpactQuery,
        _cancellation: CancellationToken,
    ) -> Result<String, AppError> {
        Err(AppError::new(
            nexus_model::ErrorCode::Policy,
            "Systemd stop-impact inspection is unavailable on this transport.",
        ))
    }

    async fn disconnect(&self) -> Result<(), AppError>;

    fn is_closed(&self) -> bool;
}
