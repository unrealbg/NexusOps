use crate::{AppError, HostId, HostSessionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One bounded, read-only observation for a single connected SSH session.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostMonitorSample {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub cpu_usage_percent: Option<f64>,
    #[ts(as = "Option<f64>")]
    pub memory_total_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub memory_used_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub swap_total_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub swap_used_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub root_total_bytes: Option<u64>,
    #[ts(as = "Option<f64>")]
    pub root_used_bytes: Option<u64>,
    pub network_rx_bytes_per_second: Option<f64>,
    pub network_tx_bytes_per_second: Option<f64>,
    pub warnings: Vec<AppError>,
}
