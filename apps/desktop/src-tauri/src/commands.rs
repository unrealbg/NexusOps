use nexus_core::Application;
use nexus_model::{
    AppError, Host, HostId, HostInput, HostKeyChallenge, HostSession, HostSessionId,
    TerminalOutputBatch, TerminalSession, TerminalSessionId, TerminalSize,
};
use nexus_secrets::CredentialInput;
use tauri::State;

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
