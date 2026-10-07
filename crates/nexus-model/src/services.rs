use crate::{HostId, HostSessionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct ServiceObservationId(pub Uuid);

impl ServiceObservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ServiceObservationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct RemoteOperationPlanId(pub Uuid);

impl RemoteOperationPlanId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RemoteOperationPlanId {
    fn default() -> Self {
        Self::new()
    }
}

/// Validated remote display data from one fixed, read-only systemd observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEntry {
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub description: String,
    pub can_reload: bool,
    #[serde(default)]
    pub reset_failed_observation_id: Option<ServiceObservationId>,
    #[serde(default)]
    pub try_restart_observation_id: Option<ServiceObservationId>,
    #[serde(default)]
    pub reload_observation_id: Option<ServiceObservationId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub entries: Vec<ServiceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceResetFailedPlan {
    pub plan_id: RemoteOperationPlanId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub risk: crate::OperationRisk,
    pub expires_in_seconds: u64,
    pub effect: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceResetFailedOutcome {
    Success,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceResetFailedAuditStatus {
    Persisted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceResetFailedPostObservationStatus {
    Refreshed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceResetFailedResult {
    pub outcome: ServiceResetFailedOutcome,
    pub audit_status: ServiceResetFailedAuditStatus,
    pub post_observation_status: ServiceResetFailedPostObservationStatus,
    pub snapshot: Option<ServiceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTryRestartPlan {
    pub plan_id: RemoteOperationPlanId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub risk: crate::OperationRisk,
    pub expires_in_seconds: u64,
    pub effect: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceTryRestartOutcome {
    Success,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceTryRestartAuditStatus {
    Persisted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceTryRestartPostObservationStatus {
    Refreshed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceTryRestartResult {
    pub outcome: ServiceTryRestartOutcome,
    pub audit_status: ServiceTryRestartAuditStatus,
    pub post_observation_status: ServiceTryRestartPostObservationStatus,
    pub snapshot: Option<ServiceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceReloadPlan {
    pub plan_id: RemoteOperationPlanId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub unit: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub can_reload: bool,
    pub risk: crate::OperationRisk,
    pub expires_in_seconds: u64,
    pub effect: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceReloadOutcome {
    Success,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceReloadAuditStatus {
    Persisted,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ServiceReloadPostObservationStatus {
    Refreshed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServiceReloadResult {
    pub outcome: ServiceReloadOutcome,
    pub audit_status: ServiceReloadAuditStatus,
    pub post_observation_status: ServiceReloadPostObservationStatus,
    pub snapshot: Option<ServiceSnapshot>,
}
