//! Transport-independent domain and IPC data. Credentials deliberately live elsewhere.
mod error;
mod host;
mod monitoring;
mod services;
mod session;
mod sftp;
mod terminal;
pub use error::*;
pub use host::*;
pub use monitoring::*;
pub use services::*;
pub use session::*;
pub use sftp::*;
pub use terminal::*;

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
