mod client;
mod path;
mod policy;
mod transfer;

pub use client::{EntryIdentity, Progress, RawSftpClient, SftpClient};
pub use path::{
    display_name, join_remote, parent_remote, validate_child_name, validate_remote_path,
    validate_windows_file_name,
};
pub use policy::{
    FilePlanStore, InternalPlan, LocalItem, MutationSpec, PlanPayload, PlannedDownload,
    PlannedUpload, RemoteSource, validate_local_directory,
};
pub use transfer::TransferManager;

use async_trait::async_trait;
use nexus_model::{AppError, HostId, HostSessionId};
use std::sync::Arc;

#[async_trait]
pub trait SftpConnector: Send + Sync {
    async fn open_sftp(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<Arc<dyn SftpClient>, AppError>;
}
