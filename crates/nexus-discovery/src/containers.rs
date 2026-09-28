use chrono::Utc;
use nexus_model::{
    AppError, ContainerEntry, ContainerProvider, ContainerSnapshot, ContainerState, ErrorCode,
    HostId, HostSessionId,
};
use nexus_operations::{OperationEngine, ReadOnlyCommand, RemoteSession};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

const MAX_RECORDS: usize = 64;
const MAX_RAW_BYTES: usize = 64 * 1024;

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::Discovery,
        "Docker container inventory is unavailable for this connection.",
    )
}

// serde rejects duplicate and missing required fields. Unknown fields fail closed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContainer {
    id: String,
    image: String,
    name: String,
    state: ContainerState,
    status: String,
    ports: String,
    networks: String,
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

fn display_text(value: &str, max_bytes: usize, required: bool) -> Result<(), AppError> {
    if (required && value.is_empty())
        || value.len() > max_bytes
        || value.chars().any(unsafe_display)
    {
        return Err(unavailable());
    }
    Ok(())
}

impl TryFrom<RawContainer> for ContainerEntry {
    type Error = AppError;

    fn try_from(raw: RawContainer) -> Result<Self, Self::Error> {
        if raw.id.len() != 64
            || !raw
                .id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(unavailable());
        }
        display_text(&raw.name, 255, true)?;
        display_text(&raw.image, 512, true)?;
        display_text(&raw.status, 512, true)?;
        display_text(&raw.ports, 1024, false)?;
        display_text(&raw.networks, 1024, false)?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            image: raw.image,
            state: raw.state,
            status: raw.status,
            ports: raw.ports,
            networks: raw.networks,
        })
    }
}

/// Bounded JSON-lines projection. A bad record invalidates the whole snapshot.
pub fn parse_docker_containers(output: &str) -> Result<Vec<ContainerEntry>, AppError> {
    if output.len() > MAX_RAW_BYTES || output.contains('\0') {
        return Err(unavailable());
    }
    let mut entries = Vec::new();
    for line in output.split_terminator('\n') {
        if entries.len() == MAX_RECORDS || !line.trim_start().starts_with('{') {
            return Err(unavailable());
        }
        let raw: RawContainer = serde_json::from_str(line).map_err(|_| unavailable())?;
        entries.push(ContainerEntry::try_from(raw)?);
    }
    Ok(entries)
}

pub async fn observe_docker_containers(
    session: &dyn RemoteSession,
    cancellation: CancellationToken,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<ContainerSnapshot, AppError> {
    let engine = OperationEngine::default();
    let output = engine
        .execute(
            session,
            &engine.plan(ReadOnlyCommand::DockerContainers),
            cancellation,
        )
        .await
        .map_err(|error| AppError::new(error.code, unavailable().message))?;
    let entries = parse_docker_containers(&output)?;
    Ok(ContainerSnapshot {
        host_id,
        host_session_id,
        observed_at: Utc::now().to_rfc3339(),
        provider: ContainerProvider::DockerSystem,
        entries,
    })
}

#[cfg(test)]
mod tests;
