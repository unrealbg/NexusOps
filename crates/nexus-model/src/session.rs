use crate::{AppError, ErrorCode, HostId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    AwaitingTrust,
    Connected,
    Disconnecting,
    Failed,
}
impl ConnectionState {
    pub fn transition(self, next: Self) -> Result<Self, AppError> {
        use ConnectionState::*;
        if self == next
            || matches!(
                (self, next),
                (Disconnected | Failed | AwaitingTrust, Connecting)
                    | (
                        Connecting,
                        Connected | AwaitingTrust | Failed | Disconnecting
                    )
                    | (Connected, Disconnecting | Failed)
                    | (Disconnecting, Disconnected | Failed)
                    | (AwaitingTrust | Failed, Disconnected)
            )
        {
            Ok(next)
        } else {
            Err(AppError::new(
                ErrorCode::Conflict,
                "Invalid connection state transition.",
            ))
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostFingerprint {
    pub algorithm: String,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostIdentity {
    pub hostname: String,
    pub fingerprint: HostFingerprint,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostCapability {
    pub id: String,
    pub available: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySnapshot {
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub kernel: Option<String>,
    pub architecture: Option<String>,
    pub uptime_seconds: Option<f64>,
    pub load_one: Option<f64>,
    #[ts(as = "Option<f64>")]
    pub memory_total_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub memory_used_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub root_total_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub root_used_bytes: Option<u64>,
    pub observed_at: String,
    pub warnings: Vec<AppError>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostSession {
    pub host_id: HostId,
    pub state: ConnectionState,
    pub error: Option<AppError>,
    pub identity: Option<HostIdentity>,
    pub capabilities: Vec<HostCapability>,
    pub discovery: Option<DiscoverySnapshot>,
}
impl HostSession {
    pub fn disconnected(host_id: HostId) -> Self {
        Self {
            host_id,
            state: ConnectionState::Disconnected,
            error: None,
            identity: None,
            capabilities: vec![],
            discovery: None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_machine_rejects_skipping_verification() {
        use ConnectionState::*;
        assert!(Disconnected.transition(Connected).is_err());
        assert!(
            Disconnected
                .transition(Connecting)
                .and_then(|s| s.transition(AwaitingTrust))
                .and_then(|s| s.transition(Connecting))
                .and_then(|s| s.transition(Connected))
                .and_then(|s| s.transition(Disconnecting))
                .and_then(|s| s.transition(Disconnected))
                .is_ok()
        );
    }
}
