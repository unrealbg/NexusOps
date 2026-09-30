//! Transport-independent domain and IPC data. Credentials deliberately live elsewhere.
mod containers;
mod error;
mod host;
mod logs;
mod monitoring;
mod network;
mod security;
mod services;
mod session;
mod sftp;
mod terminal;
mod update;
pub use containers::*;
pub use error::*;
pub use host::*;
pub use logs::*;
pub use monitoring::*;
pub use network::*;
pub use security::*;
pub use services::*;
pub use session::*;
pub use sftp::*;
pub use terminal::*;
pub use update::*;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Ordered policy levels. Only ReadOnly is executable in Goal 01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OperationRisk {
    ReadOnly,
    Low,
    Moderate,
    High,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: String,
    pub kind: String,
    pub risk: OperationRisk,
}
