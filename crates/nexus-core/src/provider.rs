use async_trait::async_trait;
use nexus_model::{AppError, Host};
use nexus_operations::RemoteSession;
use nexus_secrets::Credential;
use nexus_sftp::SftpConnector;
use nexus_terminal::TerminalConnector;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The application orchestration boundary for future Local, Serial or other providers.
/// Only SSH is registered in Goal 01; domain/UI code never references russh.
pub trait ConnectedTransport: RemoteSession + TerminalConnector + SftpConnector {}
impl<T: RemoteSession + TerminalConnector + SftpConnector + ?Sized> ConnectedTransport for T {}

#[async_trait]
pub trait ConnectionProvider: Send + Sync {
    async fn connect(
        &self,
        host: &Host,
        credential: Credential,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ConnectedTransport>, AppError>;
}
#[async_trait]
impl ConnectionProvider for nexus_ssh::SshProvider {
    async fn connect(
        &self,
        host: &Host,
        credential: Credential,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ConnectedTransport>, AppError> {
        let session = nexus_ssh::SshProvider::connect(self, host, credential, cancel).await?;
        Ok(session)
    }
}
