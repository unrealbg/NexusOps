use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Validation,
    NotFound,
    Persistence,
    Dns,
    Timeout,
    Refused,
    Authentication,
    UnknownHostKey,
    ChangedHostKey,
    Discovery,
    SecureStorage,
    Cancelled,
    Connection,
    Conflict,
    Policy,
    TerminalPty,
    TerminalShell,
    TerminalUnavailable,
    TerminalResize,
    TerminalStream,
    TerminalStartup,
    TerminalChannel,
}

/// Safe user-visible failure. Never put raw library errors or remote output in message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub host_key: Option<Box<HostKeyChallenge>>,
}
impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            host_key: None,
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AppError {}

/// A server key observed during a rejected handshake. Trust must match a pending challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyChallenge {
    pub hostname: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
}
