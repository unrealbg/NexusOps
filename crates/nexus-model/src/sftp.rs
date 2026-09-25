use crate::{AppError, ErrorCode, HostId, HostSessionId, OperationRisk};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(type = "string")]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

opaque_id!(SftpSessionId);
opaque_id!(LocalGrantId);
opaque_id!(FilePlanId);
opaque_id!(TransferJobId);
opaque_id!(EditorDocumentId);

pub const MAX_REMOTE_EDITOR_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TextNewline {
    Lf,
    CrLf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTextDocument {
    pub id: EditorDocumentId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub sftp_session_id: SftpSessionId,
    pub path: String,
    pub text: String,
    pub original_bytes: u32,
    pub newline: TextNewline,
    pub bom: bool,
    pub max_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SftpExtension {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SftpLimits {
    pub max_packet_bytes: Option<String>,
    pub max_read_bytes: Option<String>,
    pub max_write_bytes: Option<String>,
    pub max_open_handles: Option<String>,
    pub client_chunk_bytes: String,
    pub listing_entry_cap: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SftpSessionInfo {
    pub id: SftpSessionId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub protocol_version: u32,
    pub extensions: Vec<SftpExtension>,
    pub limits: SftpLimits,
    pub root_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RemoteEntryKind {
    File,
    Directory,
    Symlink,
    Other,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub display_name: String,
    pub path: String,
    pub kind: RemoteEntryKind,
    pub size_bytes: Option<String>,
    pub modified_at: Option<String>,
    pub permissions: Option<String>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub sftp_session_id: SftpSessionId,
    pub path: String,
    pub entries: Vec<RemoteEntry>,
    pub partial: bool,
    pub entry_cap: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    Skip,
    KeepBoth,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LocalGrantKind {
    UploadFiles,
    DownloadDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LocalSelectionItem {
    pub display_name: String,
    pub size_bytes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LocalSelectionGrant {
    pub id: LocalGrantId,
    pub kind: LocalGrantKind,
    pub items: Vec<LocalSelectionItem>,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FileOperationKind {
    Upload,
    Download,
    CreateDirectory,
    Rename,
    Delete,
    EditText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FilePlanItem {
    pub source_display: String,
    pub destination_display: String,
    pub size_bytes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationPlan {
    pub id: FilePlanId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub sftp_session_id: SftpSessionId,
    pub kind: FileOperationKind,
    pub risk: OperationRisk,
    pub conflict_policy: Option<ConflictPolicy>,
    pub items: Vec<FilePlanItem>,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TransferState {
    Queued,
    Preparing,
    AwaitingDecision,
    Transferring,
    CancelRequested,
    Finalizing,
    Completed,
    Failed,
    Cancelled,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransferJob {
    pub id: TransferJobId,
    pub host_id: HostId,
    pub host_session_id: HostSessionId,
    pub sftp_session_id: SftpSessionId,
    pub direction: TransferDirection,
    pub source_display: String,
    pub destination_display: String,
    pub state: TransferState,
    pub confirmed_bytes: String,
    pub total_bytes: Option<String>,
    pub error: Option<AppError>,
    pub retryable: bool,
}

impl TransferJob {
    pub fn failed_policy(message: impl Into<String>) -> AppError {
        AppError::new(ErrorCode::FilePolicy, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_counters_are_lossless_json_strings() {
        let above_javascript_integer_limit = "9007199254740993";
        let job = TransferJob {
            id: TransferJobId::new(),
            host_id: HostId::new(),
            host_session_id: HostSessionId::new(),
            sftp_session_id: SftpSessionId::new(),
            direction: TransferDirection::Download,
            source_display: "binary.dat".into(),
            destination_display: "binary.dat".into(),
            state: TransferState::Transferring,
            confirmed_bytes: above_javascript_integer_limit.into(),
            total_bytes: Some("18446744073709551615".into()),
            error: None,
            retryable: false,
        };
        let json = serde_json::to_value(job).unwrap();
        assert_eq!(json["confirmedBytes"], above_javascript_integer_limit);
        assert_eq!(json["totalBytes"], "18446744073709551615");
    }
}
