use crate::{AuthenticationMethod, HostFingerprint, HostId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
