use crate::lifecycle::LifecycleCoordinator;
use crate::local_access::LocalAccessService;
use crate::update_install;
use crate::updates::UpdateService;
use nexus_core::Application;
use nexus_model::{
    AppError, ContainerSnapshot, Host, HostId, HostInput, HostKeyChallenge, HostKeyRotationPlan,
    HostKeyRotationPlanId, HostMonitorSample, HostSession, HostSessionId, NetworkSnapshot,
    ServiceSnapshot, SshEndpointTrust, SystemJournalSnapshot, TerminalOutputBatch, TerminalSession,
    TerminalSessionId, TerminalSize, UpdateAnnouncementId, UpdateOperationSnapshot,
    VerifiedArtifactId,
};
use nexus_model::{
    ConflictPolicy, DirectoryListing, FileOperationPlan, FilePlanId, LocalGrantId,
    LocalSelectionGrant, RemoteEntry, SftpSessionId, SftpSessionInfo, TransferJob, TransferJobId,
};
use nexus_secrets::CredentialInput;
use serde::Deserialize;
use tauri::State;

#[tauri::command]
pub async fn check_for_update(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: tauri::AppHandle,
    service: State<'_, UpdateService>,
) -> Result<UpdateOperationSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    service.check(&app).await
}

#[tauri::command]
pub fn get_update_state(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: tauri::AppHandle,
    service: State<'_, UpdateService>,
) -> Result<UpdateOperationSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    service.snapshot(app.package_info().version.to_string())
}

#[tauri::command]
pub async fn download_announced_update(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: tauri::AppHandle,
    service: State<'_, UpdateService>,
    announcement_id: UpdateAnnouncementId,
) -> Result<UpdateOperationSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    service
        .download(app.package_info().version.to_string(), announcement_id)
        .await
}

#[tauri::command]
pub async fn install_verified_update(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: tauri::AppHandle,
    service: State<'_, UpdateService>,
    verified_artifact_id: VerifiedArtifactId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    service.validate_install(verified_artifact_id)?;
    _permit.seal_and_drain_others().await?;
    let authority = service.consume_install(verified_artifact_id)?;
    update_install::finish_install(app, authority).await
}

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
pub fn list_hosts(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
) -> Result<Vec<Host>, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_hosts()
}
#[tauri::command]
pub async fn save_host(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    input: HostInput,
    credential: Option<CredentialInput>,
) -> Result<Host, AppError> {
    let _permit = lifecycle.admit()?;
    app.save_host(input, credential).await
}
#[tauri::command]
pub async fn delete_host(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.delete_host(host_id).await
}
#[tauri::command]
pub async fn connect_host(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.connect_host(host_id).await
}
#[tauri::command]
pub async fn disconnect_host(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    host_id: HostId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    local.revoke_host(host_id);
    app.disconnect_host(host_id).await
}
#[tauri::command]
pub async fn reconnect_host(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    host_id: HostId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    local.revoke_host(host_id);
    app.reconnect_host(host_id).await
}
#[tauri::command]
pub async fn get_session(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<HostSession, AppError> {
    let _permit = lifecycle.admit()?;
    app.get_session(host_id).await
}
#[tauri::command]
pub async fn get_host_ssh_trust(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<SshEndpointTrust, AppError> {
    let _permit = lifecycle.admit()?;
    app.get_host_ssh_trust(host_id).await
}
#[tauri::command]
pub async fn trust_host_key(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    challenge: HostKeyChallenge,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.trust_host_key(host_id, challenge).await
}
#[tauri::command]
pub async fn plan_host_key_rotation(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<HostKeyRotationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    app.plan_host_key_rotation(host_id).await
}
#[tauri::command]
pub async fn execute_host_key_rotation(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    plan_id: HostKeyRotationPlanId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.execute_host_key_rotation(host_id, plan_id).await
}
#[tauri::command]
pub async fn refresh_host(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.refresh_host(host_id).await
}

#[tauri::command]
pub async fn sample_host_monitor(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<HostMonitorSample, AppError> {
    let _permit = lifecycle.admit()?;
    app.sample_host_monitor(host_id, host_session_id).await
}

#[tauri::command]
pub async fn list_host_services(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<ServiceSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_host_services(host_id, host_session_id).await
}

#[tauri::command]
pub async fn list_host_network(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<NetworkSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_host_network(host_id, host_session_id).await
}

#[tauri::command]
pub async fn list_host_logs(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<SystemJournalSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_host_logs(host_id, host_session_id).await
}

#[tauri::command]
pub async fn list_host_containers(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
) -> Result<ContainerSnapshot, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_host_containers(host_id, host_session_id).await
}

#[tauri::command]
pub async fn list_terminals(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<Vec<TerminalSession>, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_terminals(host_id).await
}

#[tauri::command]
pub async fn open_terminal(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    size: TerminalSize,
) -> Result<TerminalSession, AppError> {
    let _permit = lifecycle.admit()?;
    app.open_terminal(host_id, size).await
}

#[tauri::command]
pub async fn poll_terminal(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
) -> Result<TerminalOutputBatch, AppError> {
    let _permit = lifecycle.admit()?;
    app.poll_terminal(host_id, host_session_id, terminal_id)
        .await
}

#[tauri::command]
pub async fn write_terminal(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
    data_base64: String,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.write_terminal(host_id, host_session_id, terminal_id, &data_base64)
        .await
}

#[tauri::command]
pub async fn resize_terminal(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
    size: TerminalSize,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.resize_terminal(host_id, host_session_id, terminal_id, size)
        .await
}

#[tauri::command]
pub async fn rename_terminal(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
    label: String,
) -> Result<TerminalSession, AppError> {
    let _permit = lifecycle.admit()?;
    app.rename_terminal(host_id, host_session_id, terminal_id, &label)
        .await
}

#[tauri::command]
pub async fn close_terminal(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.close_terminal(host_id, host_session_id, terminal_id)
        .await
}

#[tauri::command]
pub async fn open_sftp(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
) -> Result<SftpSessionInfo, AppError> {
    let _permit = lifecycle.admit()?;
    app.open_sftp(host_id).await
}
#[tauri::command]
pub async fn list_remote_directory(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<DirectoryListing, AppError> {
    let _permit = lifecycle.admit()?;
    app.list_remote_directory(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn remote_properties(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<RemoteEntry, AppError> {
    let _permit = lifecycle.admit()?;
    app.remote_properties(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn choose_upload_files(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
) -> Result<Option<LocalSelectionGrant>, AppError> {
    let _permit = lifecycle.admit()?;
    let scope = crate::local_access::GrantScope {
        host_id,
        host_session_id,
        sftp_session_id,
    };
    app.validate_sftp_session(host_id, host_session_id, sftp_session_id)
        .await?;
    let grant = local.choose_upload_files(scope).await?;
    if let Err(error) = app
        .validate_sftp_session(host_id, host_session_id, sftp_session_id)
        .await
    {
        local.revoke_session(host_id, host_session_id);
        return Err(error);
    }
    Ok(grant)
}
#[tauri::command]
pub async fn choose_download_directory(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
) -> Result<Option<LocalSelectionGrant>, AppError> {
    let _permit = lifecycle.admit()?;
    let scope = crate::local_access::GrantScope {
        host_id,
        host_session_id,
        sftp_session_id,
    };
    app.validate_sftp_session(host_id, host_session_id, sftp_session_id)
        .await?;
    let grant = local.choose_download_directory(scope).await?;
    if let Err(error) = app
        .validate_sftp_session(host_id, host_session_id, sftp_session_id)
        .await
    {
        local.revoke_session(host_id, host_session_id);
        return Err(error);
    }
    Ok(grant)
}
#[tauri::command]
pub async fn plan_upload(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    request: PlanUploadRequest,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    let scope = crate::local_access::GrantScope {
        host_id: request.host_id,
        host_session_id: request.host_session_id,
        sftp_session_id: request.sftp_session_id,
    };
    let sources = local.consume_upload(request.grant_id, scope)?;
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
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    request: PlanDownloadRequest,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    let scope = crate::local_access::GrantScope {
        host_id: request.host_id,
        host_session_id: request.host_session_id,
        sftp_session_id: request.sftp_session_id,
    };
    let directory = local.consume_download_directory(request.grant_id, scope)?;
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
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    parent: String,
    name: String,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    app.plan_create_directory(host_id, host_session_id, sftp_session_id, parent, name)
        .await
}
#[tauri::command]
pub async fn plan_rename(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    source: String,
    new_name: String,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    app.plan_rename(host_id, host_session_id, sftp_session_id, source, new_name)
        .await
}
#[tauri::command]
pub async fn plan_delete(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    app.plan_delete(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn open_remote_text_file(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    path: String,
) -> Result<nexus_model::RemoteTextDocument, AppError> {
    let _permit = lifecycle.admit()?;
    app.open_remote_text_file(host_id, host_session_id, sftp_session_id, path)
        .await
}
#[tauri::command]
pub async fn plan_remote_text_save(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    document_id: nexus_model::EditorDocumentId,
    text: String,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    app.plan_remote_text_save(host_id, host_session_id, sftp_session_id, document_id, text)
        .await
}
#[tauri::command]
pub async fn discard_remote_text_document(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    document_id: nexus_model::EditorDocumentId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.discard_remote_text_document(host_id, host_session_id, sftp_session_id, document_id)
}
#[tauri::command]
pub async fn execute_file_plan(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    plan_id: FilePlanId,
) -> Result<Vec<TransferJob>, AppError> {
    let _permit = lifecycle.admit()?;
    app.execute_file_plan(host_id, host_session_id, sftp_session_id, plan_id)
        .await
}
#[tauri::command]
pub async fn discard_file_plan(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    plan_id: FilePlanId,
) -> Result<bool, AppError> {
    let _permit = lifecycle.admit()?;
    app.discard_file_plan(host_id, host_session_id, sftp_session_id, plan_id)
        .await
}
#[tauri::command]
pub async fn discard_local_grant(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    local: State<'_, LocalAccessService>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    grant_id: LocalGrantId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.validate_sftp_session(host_id, host_session_id, sftp_session_id)
        .await?;
    local.discard(
        grant_id,
        crate::local_access::GrantScope {
            host_id,
            host_session_id,
            sftp_session_id,
        },
    )
}
#[tauri::command]
pub fn list_transfers(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: Option<HostId>,
) -> Result<Vec<TransferJob>, AppError> {
    let _permit = lifecycle.admit()?;
    Ok(app.list_transfers(host_id))
}
#[tauri::command]
pub async fn cancel_transfer(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    job_id: TransferJobId,
) -> Result<(), AppError> {
    let _permit = lifecycle.admit()?;
    app.cancel_transfer(host_id, host_session_id, sftp_session_id, job_id)
        .await
}
#[tauri::command]
pub async fn plan_retry_transfer(
    lifecycle: State<'_, LifecycleCoordinator>,
    app: State<'_, Application>,
    host_id: HostId,
    host_session_id: HostSessionId,
    sftp_session_id: SftpSessionId,
    job_id: TransferJobId,
) -> Result<FileOperationPlan, AppError> {
    let _permit = lifecycle.admit()?;
    app.plan_retry_transfer(host_id, host_session_id, sftp_session_id, job_id)
        .await
}
