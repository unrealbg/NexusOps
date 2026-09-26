use crate::{HostId, HostSessionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Validated remote display data from one fixed, read-only systemd observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEntry {
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub entries: Vec<ServiceEntry>,
}
