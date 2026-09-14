use nexus_model::{AppError, ErrorCode, HostId, Operation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditActor {
    User,
    AI,
    Plugin,
    Automation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditOutcome {
    Success,
    Failed,
    Cancelled,
}

/// Deliberately closed metadata schema: no error text, credential, command or output field.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditEvent {
    pub host: HostId,
    pub timestamp: String,
    pub operation: Operation,
    pub actor: AuditActor,
    pub outcome: AuditOutcome,
    pub duration_ms: u64,
}

impl AuditEvent {
    pub fn new(
        host: HostId,
        operation: Operation,
        actor: AuditActor,
        outcome: AuditOutcome,
        duration_ms: u64,
    ) -> Self {
        Self {
            host,
            timestamp: chrono::Utc::now().to_rfc3339(),
            operation,
            actor,
            outcome,
            duration_ms,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if chrono::DateTime::parse_from_rfc3339(&self.timestamp).is_err()
            || !metadata_identifier(&self.operation.id)
            || !metadata_identifier(&self.operation.kind)
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Audit metadata contains an invalid identifier or timestamp.",
            ));
        }
        Ok(())
    }
}

fn metadata_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}
