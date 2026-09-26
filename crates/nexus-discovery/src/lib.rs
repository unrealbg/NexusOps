//! Independent, bounded Linux probes. Untrusted remote output is only parsed as data.

mod capabilities;
mod monitoring;
mod parsers;
mod probes;
mod services;

pub use capabilities::{CapabilityRegistry, capabilities};
pub use monitoring::{MonitorBaseline, MonitorReading, derive_sample, observe_monitor};
pub use probes::{HostProbe, builtin_probes};
pub use services::{observe_services, parse_services};

use nexus_model::{AppError, DiscoverySnapshot, ErrorCode};
use nexus_operations::{OperationEngine, RemoteSession};
use tokio_util::sync::CancellationToken;

/// Discover a connected host. Optional failures become warnings; only cancellation
/// aborts discovery. A failed probe never disconnects the underlying session.
pub async fn discover(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
) -> Result<DiscoverySnapshot, AppError> {
    let mut snapshot = DiscoverySnapshot {
        hostname: None,
        os: None,
        os_version: None,
        kernel: None,
        architecture: None,
        uptime_seconds: None,
        load_one: None,
        memory_total_bytes: None,
        memory_used_bytes: None,
        root_total_bytes: None,
        root_used_bytes: None,
        observed_at: chrono::Utc::now().to_rfc3339(),
        warnings: Vec::new(),
    };
    let engine = OperationEngine::default();
    for probe in builtin_probes() {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let plan = engine.plan(probe.command());
        let result = match engine.execute(session, &plan, cancellation.clone()).await {
            Ok(output) => probe.parse(&output, &mut snapshot),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            if error.code == ErrorCode::Cancelled || cancellation.is_cancelled() {
                return Err(cancelled());
            }
            tracing::warn!(probe = probe.id(), error_code = ?error.code, "Optional discovery probe failed");
            snapshot.warnings.push(AppError::new(
                error.code,
                format!("{} discovery is unavailable.", probe.id()),
            ));
        }
    }
    if cancellation.is_cancelled() {
        return Err(cancelled());
    }
    snapshot.observed_at = chrono::Utc::now().to_rfc3339();
    Ok(snapshot)
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Host discovery was cancelled.")
}

#[cfg(test)]
mod tests;
