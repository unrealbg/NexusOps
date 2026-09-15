use crate::local_access::LocalAccessService;
use nexus_core::Application;
use nexus_model::{
    AppError, Host, HostId, HostInput, HostKeyChallenge, HostSession, HostSessionId,
    TerminalOutputBatch, TerminalSession, TerminalSessionId, TerminalSize,
};
use nexus_model::{
    ConflictPolicy, DirectoryListing, FileOperationPlan, FilePlanId, LocalGrantId,
    LocalSelectionGrant, RemoteEntry, SftpSessionId, SftpSessionInfo, TransferJob, TransferJobId,
};
use nexus_secrets::CredentialInput;
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUploadRequest {
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    grant_id: LocalGrantId,
    remote_directory: String,
    conflict_policy: ConflictPolicy,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDownloadRequest {
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    grant_id: LocalGrantId,
    remote_paths: Vec<String>,
    conflict_policy: ConflictPolicy,
}

#[tauri::command]
pub fn list_hosts(app: State<'_, Application>) -> Result<Vec<Host>, AppError> {
    app.list_hosts()
}
#[tauri::command]
pub async fn save_host(
    app: State<'_, Application>,
    input: HostInput,
    credential: Option<CredentialInput>,
) -> Result<Host, AppError> {
    app.save_host(input, credential).await
}
#[tauri::command]
pub async fn delete_host(app: State<'_, Application>, host_id: HostId) -> Result<(), AppError> {
    app.delete_host(host_id).await
}
#[tauri::command]
pub async fn connect_host(app: State<'_, Application>, host_id: HostId) -> Result<(), AppError> {
    app.connect_host(host_id).await
}
#[tauri::command]
pub async fn disconnect_host(app: State<'_, Application>, host_id: HostId) -> Result<(), AppError> {
    app.disconnect_host(host_id).await
}
#[tauri::command]
pub async fn reconnect_host(app: State<'_, Application>, host_id: HostId) -> Result<(), AppError> {
    app.reconnect_host(host_id).await
}
#[tauri::command]
pub async fn get_session(
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<HostSession, AppError> {
    app.get_session(host_id).await
}
#[tauri::command]
pub async fn trust_host_key(
    app: State<'_, Application>,
    host_id: HostId,
    challenge: HostKeyChallenge,
) -> Result<(), AppError> {
    app.trust_host_key(host_id, challenge).await
}
#[tauri::command]
pub async fn refresh_host(app: State<'_, Application>, host_id: HostId) -> Result<(), AppError> {
    app.refresh_host(host_id).await
}

#[tauri::command]
pub async fn list_terminals(
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<Vec<TerminalSession>, AppError> {
    app.list_terminals(host_id).await
}

#[tauri::command]
pub async fn open_terminal(
    app: State<'_, Application>,
    host_id: HostId,
    size: TerminalSize,
) -> Result<TerminalSession, AppError> {
    app.open_terminal(host_id, size).await
}

#[tauri::command]
pub async fn poll_terminal(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
) -> Result<TerminalOutputBatch, AppError> {
    app.poll_terminal(host_id, host_session_id, terminal_id)
        .await
}

#[tauri::command]
pub async fn write_terminal(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
    data_base64: String,
) -> Result<(), AppError> {
    app.write_terminal(host_id, host_session_id, terminal_id, &data_base64)
        .await
}

#[tauri::command]
pub async fn resize_terminal(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
    size: TerminalSize,
) -> Result<(), AppError> {
    app.resize_terminal(host_id, host_session_id, terminal_id, size)
        .await
}

#[tauri::command]
pub async fn rename_terminal(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
    label: String,
) -> Result<TerminalSession, AppError> {
    app.rename_terminal(host_id, host_session_id, terminal_id, &label)
        .await
}

#[tauri::command]
pub async fn close_terminal(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
) -> Result<(), AppError> {
    app.close_terminal(host_id, host_session_id, terminal_id)
        .await
}

#[tauri::command]
pub async fn open_sftp(
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<SftpSessionInfo, AppError> {
    app.open_sftp(host_id).await
}
#[tauri::command]
pub async fn list_remote_directory(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<DirectoryListing, AppError> {
    app.list_remote_directory(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn remote_properties(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<RemoteEntry, AppError> {
    app.remote_properties(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn choose_upload_files(
    local: State<'_, LocalAccessService>,
) -> Result<Option<LocalSelectionGrant>, AppError> {
    local.choose_upload_files().await
}
#[tauri::command]
pub async fn choose_download_directory(
    local: State<'_, LocalAccessService>,
) -> Result<Option<LocalSelectionGrant>, AppError> {
    local.choose_download_directory().await
}
#[tauri::command]
pub async fn plan_upload(
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    request: PlanUploadRequest,
) -> Result<FileOperationPlan, AppError> {
    let sources = local.consume_upload(request.grant_id)?;
    app.plan_upload(
        request.host_id,
        request.host_session_id,
        request.sftp_session_id,
        sources,
        request.remote_directory,
        request.conflict_policy,
    )
    .await
}
#[tauri::command]
pub async fn plan_download(
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    request: PlanDownloadRequest,
) -> Result<FileOperationPlan, AppError> {
    let directory = local.consume_download_directory(request.grant_id)?;
    app.plan_download(
        request.host_id,
        request.host_session_id,
        request.sftp_session_id,
        request.remote_paths,
        directory,
        request.conflict_policy,
    )
    .await
}
#[tauri::command]
pub async fn plan_create_directory(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    parent: String,
    name: String,
) -> Result<FileOperationPlan, AppError> {
    app.plan_create_directory(host_id, host_session_id, sftp_session_id, parent, name)
        .await
}
#[tauri::command]
pub async fn plan_rename(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    source: String,
    new_name: String,
) -> Result<FileOperationPlan, AppError> {
    app.plan_rename(host_id, host_session_id, sftp_session_id, source, new_name)
        .await
}
#[tauri::command]
pub async fn plan_delete(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<FileOperationPlan, AppError> {
    app.plan_delete(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn execute_file_plan(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    plan_id: FilePlanId,
) -> Result<Vec<TransferJob>, AppError> {
    app.execute_file_plan(host_id, host_session_id, sftp_session_id, plan_id)
        .await
}
#[tauri::command]
pub fn list_transfers(app: State<'_, Application>, host_id: Option<HostId>) -> Vec<TransferJob> {
    app.list_transfers(host_id)
}
#[tauri::command]
pub async fn cancel_transfer(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    job_id: TransferJobId,
) -> Result<(), AppError> {
    app.cancel_transfer(host_id, host_session_id, sftp_session_id, job_id)
        .await
}
#[tauri::command]
pub async fn plan_retry_transfer(
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    job_id: TransferJobId,
) -> Result<FileOperationPlan, AppError> {
    app.plan_retry_transfer(host_id, host_session_id, sftp_session_id, job_id)
        .await
}
