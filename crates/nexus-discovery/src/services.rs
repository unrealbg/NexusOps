use nexus_model::{AppError, ErrorCode, HostId, HostSessionId, ServiceEntry, ServiceSnapshot};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const MAX_SERVICES: usize = 512;
const MAX_UNIT_BYTES: usize = 255;
const MAX_STATE_BYTES: usize = 32;
const MAX_DESCRIPTION_BYTES: usize = 512;

fn invalid() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an invalid service inventory.",
    )
}

fn unsafe_display(character: char) -> bool {
    character.is_control()
        || (character.is_whitespace() && character != ' ')
        || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        || matches!(character, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
}

fn token(value: &str, max_bytes: usize) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > max_bytes
        || value
            .chars()
            .any(|c| c.is_whitespace() || unsafe_display(c))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Parse only the five display fields of `systemctl list-units` output. No raw
/// output is retained or included in an error. The engine applies its 64 KiB cap first.
pub fn parse_services(output: &str) -> Result<Vec<ServiceEntry>, AppError> {
    let mut entries = Vec::new();
    let mut units = HashSet::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if entries.len() == MAX_SERVICES {
            return Err(invalid());
        }
        // The first four fields are ASCII-whitespace delimited. Preserve all
        // interior spacing and Unicode in the remainder (the description).
        let bytes = line.as_bytes();
        let mut offset = 0;
        let mut fields = Vec::with_capacity(4);
        for _ in 0..4 {
            while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
                offset += 1;
            }
            let start = offset;
            while offset < bytes.len() && !bytes[offset].is_ascii_whitespace() {
                offset += 1;
            }
            if start == offset || !line.is_char_boundary(start) || !line.is_char_boundary(offset) {
                return Err(invalid());
            }
            fields.push(&line[start..offset]);
        }
        let description = line[offset..].trim();
        token(fields[0], MAX_UNIT_BYTES)?;
        if !fields[0].ends_with(".service") {
            return Err(invalid());
        }
        for state in &fields[1..] {
            token(state, MAX_STATE_BYTES)?;
        }
        if description.is_empty()
            || description.len() > MAX_DESCRIPTION_BYTES
            || description.chars().any(unsafe_display)
            || !units.insert(fields[0].to_owned())
        {
            return Err(invalid());
        }
        entries.push(ServiceEntry {
            unit: fields[0].to_owned(),
            load_state: fields[1].to_owned(),
            active_state: fields[2].to_owned(),
            sub_state: fields[3].to_owned(),
            description: description.to_owned(),
        });
    }
    Ok(entries)
}

pub async fn observe_services(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<ServiceSnapshot, AppError> {
    let engine = OperationEngine::default();
    let plan = engine.plan(ReadOnlyCommand::SystemServices);
    let output = engine.execute(session, &plan, cancellation).await?;
    let entries = parse_services(&output)?;
    Ok(ServiceSnapshot {
        host_id,
        host_session_id,
        observed_at: chrono::Utc::now().to_rfc3339(),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_states_descriptions_and_empty_result() {
        assert!(parse_services("").unwrap().is_empty());
        let rows = parse_services("sshd.service loaded active running OpenSSH server daemon\nfoo.service loaded inactive dead  A service with spaces\nfail.service not-found failed failed Ошибка службы\nfuture.service new-state repairing waiting Future description\n").unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].description, "A service with spaces");
        assert_eq!(rows[2].description, "Ошибка службы");
        assert_eq!(rows[3].active_state, "repairing");
    }

    #[test]
    fn enforces_row_and_field_bounds() {
        let row = "a.service loaded active running Description\n";
        let max_unit = format!("{}.service", "a".repeat(MAX_UNIT_BYTES - ".service".len()));
        assert_eq!(max_unit.len(), MAX_UNIT_BYTES);
        assert!(parse_services(&format!("{max_unit} loaded active running Desc")).is_ok());
        assert!(
            parse_services(&format!(
                "a.service {} active running Desc",
                "s".repeat(MAX_STATE_BYTES)
            ))
            .is_ok()
        );
        assert_eq!(
            parse_services(&format!("{}\n", "a".repeat(MAX_UNIT_BYTES)))
                .unwrap_err()
                .code,
            ErrorCode::Discovery
        );
        let lines = (0..MAX_SERVICES)
            .map(|i| format!("s{i}.service loaded active running Service {i}\n"))
            .collect::<String>();
        assert_eq!(parse_services(&lines).unwrap().len(), MAX_SERVICES);
        assert!(parse_services(&(lines + row)).is_err());
        assert!(
            parse_services(&format!(
                "{} loaded active running Desc",
                "a".repeat(MAX_UNIT_BYTES + 1)
            ))
            .is_err()
        );
        assert!(
            parse_services(&format!(
                "a.service {} active running Desc",
                "x".repeat(MAX_STATE_BYTES + 1)
            ))
            .is_err()
        );
        assert!(
            parse_services(&format!(
                "a.service loaded active running {}",
                "x".repeat(MAX_DESCRIPTION_BYTES + 1)
            ))
            .is_err()
        );
        assert!(
            parse_services(&format!(
                "a.service loaded active running {}",
                "x".repeat(MAX_DESCRIPTION_BYTES)
            ))
            .is_ok()
        );
    }

    #[test]
    fn rejects_duplicates_missing_fields_and_hostile_display_data_without_echo() {
        for output in [
            "a.service loaded active running Desc\na.service loaded active running Again",
            "a.service loaded active",
            "a.service loaded active running",
            "a\x1b[31m.service loaded active running Desc",
            "a.service lo\x1b[31maded active running Desc",
            "a.service loaded active running \x1b[31mDesc",
            "a\u{202e}.service loaded active running Desc",
            "a.service loaded active running Unsafe \u{2067}description",
            "a.service loaded active running Line\rbreak",
            "bad unit.service loaded active running Description",
            "UNIT LOAD ACTIVE SUB DESCRIPTION",
        ] {
            let error = parse_services(output).expect_err("invalid output");
            assert_eq!(error.code, ErrorCode::Discovery);
            assert!(!error.message.contains(output));
        }
    }
}
