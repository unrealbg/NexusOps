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

#[derive(Default)]
struct ServiceProperties<'a> {
    id: Option<&'a str>,
    load_state: Option<&'a str>,
    active_state: Option<&'a str>,
    sub_state: Option<&'a str>,
    description: Option<&'a str>,
}

impl<'a> ServiceProperties<'a> {
    fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.load_state.is_none()
            && self.active_state.is_none()
            && self.sub_state.is_none()
            && self.description.is_none()
    }

    fn insert(&mut self, key: &str, value: &'a str) -> Result<(), AppError> {
        let field = match key {
            "Id" => &mut self.id,
            "LoadState" => &mut self.load_state,
            "ActiveState" => &mut self.active_state,
            "SubState" => &mut self.sub_state,
            "Description" => &mut self.description,
            _ => return Err(invalid()),
        };
        if field.replace(value).is_some() {
            return Err(invalid());
        }
        Ok(())
    }

    fn finish(self, units: &mut HashSet<String>) -> Result<ServiceEntry, AppError> {
        let id = self.id.ok_or_else(invalid)?;
        let load_state = self.load_state.ok_or_else(invalid)?;
        let active_state = self.active_state.ok_or_else(invalid)?;
        let sub_state = self.sub_state.ok_or_else(invalid)?;
        let description = self.description.ok_or_else(invalid)?;

        token(id, MAX_UNIT_BYTES)?;
        if !id.ends_with(".service") {
            return Err(invalid());
        }
        for state in [load_state, active_state, sub_state] {
            token(state, MAX_STATE_BYTES)?;
        }
        if description.len() > MAX_DESCRIPTION_BYTES
            || description.chars().any(unsafe_display)
            || !units.insert(id.to_owned())
        {
            return Err(invalid());
        }
        Ok(ServiceEntry {
            unit: id.to_owned(),
            load_state: load_state.to_owned(),
            active_state: active_state.to_owned(),
            sub_state: sub_state.to_owned(),
            description: description.to_owned(),
        })
    }
}

/// Parse the selected `systemctl show` properties. A blank line terminates each
/// unit's property block. No raw output is retained or included in errors; the
/// operation engine applies its 64 KiB cap before this parser runs.
pub fn parse_services(output: &str) -> Result<Vec<ServiceEntry>, AppError> {
    let mut entries = Vec::new();
    let mut units = HashSet::new();
    let mut properties = ServiceProperties::default();
    for line in output.split_terminator('\n') {
        if line.is_empty() {
            if properties.is_empty() {
                return Err(invalid());
            }
            if entries.len() == MAX_SERVICES {
                return Err(invalid());
            }
            entries.push(properties.finish(&mut units)?);
            properties = ServiceProperties::default();
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(invalid)?;
        properties.insert(key, value)?;
    }
    if !properties.is_empty() {
        if entries.len() == MAX_SERVICES {
            return Err(invalid());
        }
        entries.push(properties.finish(&mut units)?);
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

    fn block(id: &str, load: &str, active: &str, sub: &str, description: &str) -> String {
        format!(
            "Id={id}\nLoadState={load}\nActiveState={active}\nSubState={sub}\nDescription={description}\n"
        )
    }

    fn assert_invalid(output: &str) {
        let error = parse_services(output).expect_err("invalid service output must fail closed");
        assert_eq!(error.code, ErrorCode::Discovery);
        assert_eq!(
            error.message,
            "The host returned an invalid service inventory."
        );
    }

    #[test]
    fn parses_selected_properties_and_empty_inventory() {
        assert!(parse_services("").unwrap().is_empty());
        let output = [
            block(
                "sshd.service",
                "loaded",
                "active",
                "running",
                "OpenSSH server daemon",
            ),
            block("foo.service", "loaded", "inactive", "dead", ""),
            block(
                "fail.service",
                "not-found",
                "failed",
                "failed",
                "Ошибка службы",
            ),
            block(
                "future.service",
                "new-state",
                "repairing",
                "waiting",
                "Future description",
            ),
        ]
        .join("\n");
        let rows = parse_services(&output).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].unit, "sshd.service");
        assert_eq!(rows[0].description, "OpenSSH server daemon");
        assert_eq!(rows[1].description, "");
        assert_eq!(rows[2].description, "Ошибка службы");
        assert_eq!(rows[3].active_state, "repairing");
    }

    #[test]
    fn properties_may_be_reordered_and_description_may_contain_equals() {
        let output = "Description=OpenSSH = server\nSubState=running\nId=sshd.service\nActiveState=active\nLoadState=loaded\n";
        let rows = parse_services(output).unwrap();
        assert_eq!(rows[0].description, "OpenSSH = server");
        assert_eq!(rows[0].sub_state, "running");
    }

    #[test]
    fn dynamic_job_column_cannot_contaminate_structured_description() {
        // The old table could be `UNIT LOAD ACTIVE SUB JOB DESCRIPTION`:
        // ssh.service loaded active running - OpenSSH server
        // foo.service loaded activating start start Foo daemon
        let output = [
            block(
                "ssh.service",
                "loaded",
                "active",
                "running",
                "OpenSSH server",
            ),
            block("foo.service", "loaded", "activating", "start", "Foo daemon"),
        ]
        .join("\n");
        let rows = parse_services(&output).unwrap();
        assert_eq!(rows[0].description, "OpenSSH server");
        assert_eq!(rows[1].description, "Foo daemon");
        assert_invalid("foo.service loaded activating start start Foo daemon\n");
    }

    #[test]
    fn enforces_record_and_field_bounds() {
        let max_unit = format!("{}.service", "a".repeat(MAX_UNIT_BYTES - ".service".len()));
        assert_eq!(max_unit.len(), MAX_UNIT_BYTES);
        assert!(parse_services(&block(&max_unit, "loaded", "active", "running", "Desc")).is_ok());
        assert!(
            parse_services(&block(
                "a.service",
                &"s".repeat(MAX_STATE_BYTES),
                "active",
                "running",
                "Desc"
            ))
            .is_ok()
        );
        assert!(
            parse_services(&block(
                "a.service",
                "loaded",
                "active",
                "running",
                &"x".repeat(MAX_DESCRIPTION_BYTES)
            ))
            .is_ok()
        );

        let entries = (0..MAX_SERVICES)
            .map(|i| {
                block(
                    &format!("s{i}.service"),
                    "loaded",
                    "active",
                    "running",
                    "Desc",
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_services(&entries).unwrap().len(), MAX_SERVICES);
        assert_invalid(
            &(entries + "\n" + &block("extra.service", "loaded", "active", "running", "Desc")),
        );
        assert_invalid(&block(
            &format!("{}.service", "a".repeat(MAX_UNIT_BYTES)),
            "loaded",
            "active",
            "running",
            "Desc",
        ));
        assert_invalid(&block(
            "a.service",
            "loaded",
            &"s".repeat(MAX_STATE_BYTES + 1),
            "running",
            "Desc",
        ));
        assert_invalid(&block(
            "a.service",
            "loaded",
            "active",
            "running",
            &"x".repeat(MAX_DESCRIPTION_BYTES + 1),
        ));
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_properties() {
        let valid = block("a.service", "loaded", "active", "running", "Desc");
        for output in [
            "Id=a.service\nId=b.service\nLoadState=loaded\nActiveState=active\nSubState=running\nDescription=Desc\n".to_owned(),
            format!("{valid}\n{valid}"),
            "Id=a.service\nLoadState=loaded\nActiveState=active\nDescription=Desc\n".to_owned(),
            format!("{valid}Job=start\n"),
            "Id=a.service\nLoadState=loaded\nActiveState=active\nSubState=running\nDescription\n".to_owned(),
            format!("{valid}\n\n{valid}"),
        ] {
            assert_invalid(&output);
        }
    }

    #[test]
    fn rejects_invalid_identifiers_states_and_hostile_display_data_without_echo() {
        for output in [
            block("", "loaded", "active", "running", "Desc"),
            block("not-a-service.timer", "loaded", "active", "running", "Desc"),
            block("bad unit.service", "loaded", "active", "running", "Desc"),
            block("a.service", "", "active", "running", "Desc"),
            block("a.service", "loaded", "active running", "running", "Desc"),
            block("a\x1b[31m.service", "loaded", "active", "running", "Desc"),
            block("a.service", "lo\x1b[31maded", "active", "running", "Desc"),
            block("a.service", "loaded", "active", "running", "\x1b[31mDesc"),
            block("a\u{202e}.service", "loaded", "active", "running", "Desc"),
            block("a.service", "loaded", "active", "running", "Unsafe \u{2067}description"),
            block("a.service", "loaded", "active", "running", "Zero\u{200b}width"),
            block("a.service", "loaded", "active", "running", "Line\rbreak"),
            block("a.service", "loaded", "active", "running", "Line\nbreak"),
            "Id=a.service\r\nLoadState=loaded\nActiveState=active\nSubState=running\nDescription=Desc\n".to_owned(),
        ] {
            assert_invalid(&output);
        }
    }
}
