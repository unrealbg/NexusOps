use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, JournalPriority, SystemJournalEntry,
    SystemJournalMessageState, SystemJournalSnapshot,
};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::fmt::Write;
use tokio_util::sync::CancellationToken;

const MAX_RECORDS: usize = 10;
const MAX_RAW_BYTES: usize = 64 * 1024;
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_DISPLAY_BYTES: usize = 8192;

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "System journal entries are unavailable for this connection.",
    )
}

#[derive(Default)]
enum Field {
    #[default]
    Missing,
    Present(Value),
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(Self::Present)
    }
}

// Selected duplicate JSON keys fail closed (including timestamp). Official
// journal multi-values are arrays, not duplicate JSON keys. Unknown fields are
// ignored, so cursors, boot IDs and unrelated data never become domain fields.
#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "__REALTIME_TIMESTAMP")]
    timestamp: String,
    #[serde(default, rename = "MESSAGE")]
    message: Field,
    #[serde(default, rename = "PRIORITY")]
    priority: Field,
    #[serde(default, rename = "_SYSTEMD_UNIT")]
    unit: Field,
    #[serde(default, rename = "SYSLOG_IDENTIFIER")]
    identifier: Field,
}

fn unsafe_display(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{115f}' | '\u{1160}' |
            '\u{17b4}' | '\u{17b5}' | '\u{180b}'..='\u{180f}' |
            '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' |
            '\u{2060}'..='\u{206f}' | '\u{3164}' | '\u{fe00}'..='\u{fe0f}' |
            '\u{feff}' | '\u{ffa0}' | '\u{fff9}'..='\u{fffb}' |
            '\u{1bca0}'..='\u{1bca3}' | '\u{1d173}'..='\u{1d17a}' |
            '\u{e0000}'..='\u{e0fff}')
}

fn timestamp(raw: &str) -> Result<String, AppError> {
    if raw.is_empty() || raw.len() > 20 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(unavailable());
    }
    let micros: u64 = raw.parse().map_err(|_| unavailable())?;
    let date = DateTime::from_timestamp_micros(i64::try_from(micros).map_err(|_| unavailable())?)
        .filter(|date| date.year() <= 9999)
        .ok_or_else(unavailable)?;
    Ok(date.to_rfc3339_opts(SecondsFormat::Micros, true))
}

fn optional_text(field: Field, bound: usize, token: bool) -> Result<Option<String>, AppError> {
    let value = match field {
        Field::Missing | Field::Present(Value::Null | Value::Array(_) | Value::Object(_)) => {
            return Ok(None);
        }
        Field::Present(Value::String(value)) => value,
        _ => return Err(unavailable()),
    };
    if value.is_empty()
        || value.len() > bound
        || value
            .chars()
            .any(|c| unsafe_display(c) || (token && c.is_whitespace()))
    {
        return Err(unavailable());
    }
    Ok(Some(value))
}

fn priority(field: Field) -> Result<Option<JournalPriority>, AppError> {
    let Field::Present(value) = field else {
        return Ok(None);
    };
    let text = match value {
        Value::Null | Value::Array(_) | Value::Object(_) => return Ok(None),
        Value::String(text) => text,
        _ => return Err(unavailable()),
    };
    Ok(Some(match text.as_str() {
        "0" => JournalPriority::Emergency,
        "1" => JournalPriority::Alert,
        "2" => JournalPriority::Critical,
        "3" => JournalPriority::Error,
        "4" => JournalPriority::Warning,
        "5" => JournalPriority::Notice,
        "6" => JournalPriority::Info,
        "7" => JournalPriority::Debug,
        _ => return Err(unavailable()),
    }))
}

fn message(field: Field) -> Result<(SystemJournalMessageState, Option<String>), AppError> {
    let value = match field {
        Field::Missing => return Ok((SystemJournalMessageState::Missing, None)),
        Field::Present(Value::String(value)) => value,
        Field::Present(_) => return Ok((SystemJournalMessageState::Omitted, None)),
    };
    if value.len() > MAX_MESSAGE_BYTES {
        return Err(unavailable());
    }
    let mut display = String::new();
    for c in value.chars() {
        match c {
            '\n' => display.push_str("\\n"),
            '\r' => display.push_str("\\r"),
            '\t' => display.push_str("\\t"),
            '\\' => display.push_str("\\\\"),
            c if unsafe_display(c) => {
                write!(display, "\\u{{{:04x}}}", u32::from(c)).map_err(|_| unavailable())?
            }
            c => display.push(c),
        }
        if display.len() > MAX_DISPLAY_BYTES {
            return Ok((SystemJournalMessageState::Omitted, None));
        }
    }
    Ok((SystemJournalMessageState::Text, Some(display)))
}

/// Bounded JSON-lines projection. Never stringify arbitrary values or include
/// serde errors/remote data in diagnostics. No partial snapshot on invalid input.
pub fn parse_system_journal(output: &str) -> Result<Vec<SystemJournalEntry>, AppError> {
    if output.len() > MAX_RAW_BYTES || output.contains('\0') {
        return Err(unavailable());
    }
    let mut entries = Vec::new();
    for line in output.split_terminator('\n') {
        if entries.len() == MAX_RECORDS || !line.trim_start().starts_with('{') {
            return Err(unavailable());
        }
        let raw: RawEntry = serde_json::from_str(line).map_err(|_| unavailable())?;
        let (message_state, message) = message(raw.message)?;
        entries.push(SystemJournalEntry {
            timestamp: timestamp(&raw.timestamp)?,
            priority: priority(raw.priority)?,
            unit: optional_text(raw.unit, 255, true)?,
            identifier: optional_text(raw.identifier, 128, false)?,
            message_state,
            message,
        });
    }
    Ok(entries)
}

pub async fn observe_system_journal(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<SystemJournalSnapshot, AppError> {
    let engine = OperationEngine::default();
    let output = engine
        .execute(
            session,
            &engine.plan(ReadOnlyCommand::SystemJournal),
            cancellation,
        )
        .await
        .map_err(|error| AppError::new(error.code, unavailable().message))?;
    let entries = parse_system_journal(&output)?;
    Ok(SystemJournalSnapshot {
        host_id,
        host_session_id,
        observed_at: Utc::now().to_rfc3339(),
        entries,
    })
}

#[cfg(test)]
mod tests;
