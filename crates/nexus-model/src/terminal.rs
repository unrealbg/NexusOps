use crate::{AppError, ErrorCode, HostId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct HostSessionId(pub Uuid);

impl HostSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for HostSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for HostSessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct TerminalSessionId(pub Uuid);

impl TerminalSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TerminalSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TerminalSessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    pub columns: u32,
    pub rows: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

impl TerminalSize {
    pub fn validate(self) -> Result<Self, AppError> {
        if !(2..=1_000).contains(&self.columns)
            || !(1..=1_000).contains(&self.rows)
            || self.pixel_width > 100_000
            || self.pixel_height > 100_000
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Terminal dimensions are outside the supported range.",
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TerminalState {
    Creating,
    Open,
    Closing,
    Closed,
    Failed,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub id: TerminalSessionId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub label: String,
    pub state: TerminalState,
    pub size: TerminalSize,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputBatch {
    pub session: TerminalSession,
    /// Base64 preserves arbitrary PTY bytes across JSON IPC. Chunks remain distinct.
    pub chunks_base64: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_have_explicit_bounds() {
        assert!(
            TerminalSize {
                columns: 80,
                rows: 24,
                pixel_width: 800,
                pixel_height: 480,
            }
            .validate()
            .is_ok()
        );
        assert_eq!(
            TerminalSize {
                columns: 1,
                rows: 24,
                pixel_width: 0,
                pixel_height: 0,
            }
            .validate()
            .expect_err("one column is invalid")
            .code,
            ErrorCode::Validation
        );
    }
}
