use crate::{AuthenticationMethod, HostFingerprint, HostId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct HostKeyRotationPlanId(pub Uuid);

impl HostKeyRotationPlanId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for HostKeyRotationPlanId {
    fn default() -> Self {
        Self::new()
    }
}

/// Display-only details of a backend-owned, one-shot changed-key rotation plan.
/// Expiry is Unix milliseconds for display; backend authority uses a monotonic clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyRotationPlan {
    pub id: HostKeyRotationPlanId,
    pub host_id: HostId,
    pub hostname: String,
    pub port: u16,
    pub current_fingerprint: HostFingerprint,
    pub presented_fingerprint: HostFingerprint,
    #[ts(type = "number")]
    pub expires_at_unix_ms: u64,
}

/// Local endpoint trust metadata, independent of any active SSH session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SshEndpointTrust {
    pub host_id: HostId,
    pub hostname: String,
    pub port: u16,
    pub authentication: AuthenticationMethod,
    pub endpoint_pin: Option<HostFingerprint>,
}
