use crate::{HostId, HostSessionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Docker through the SSH host's standard system Unix socket only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ContainerProvider {
    DockerSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum ContainerState {
    Created,
    Restarting,
    Running,
    Removing,
    Paused,
    Exited,
    Dead,
}

/// Untrusted Docker metadata, validated before it reaches the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ContainerEntry {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    pub status: String,
    pub ports: String,
    pub networks: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSnapshot {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub observed_at: String,
    pub provider: ContainerProvider,
    pub entries: Vec<ContainerEntry>,
}
