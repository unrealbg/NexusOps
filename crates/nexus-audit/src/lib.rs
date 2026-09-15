//! Local structured audit records contain metadata only, never command output or credentials.

mod event;
mod storage;

pub use event::{AuditActor, AuditEvent, AuditOutcome};
pub use storage::AuditLog;
