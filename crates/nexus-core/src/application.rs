use crate::{ConnectionProvider, HostRepository, StoredHost, sessions::SessionSlot};
use nexus_audit::{AuditActor, AuditEvent, AuditLog, AuditOutcome};
use nexus_model::*;
use nexus_secrets::{CredentialInput, EncryptedSecretStore, PlatformKeyProvider, SecretStore};
use nexus_sftp::{FilePlanStore, SftpClient, TransferManager};
use nexus_ssh::{KnownHosts, SshProvider};
use nexus_terminal::TerminalManager;
use std::{collections::HashMap, fs::File, path::Path, sync::Arc, time::Instant};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Typed application facade. All connection mutations are serialized just long enough
/// to update local state; network work never holds the metadata gate.
pub struct Application {
    pub(crate) repository: Arc<HostRepository>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) provider: Arc<dyn ConnectionProvider>,
    pub(crate) known_hosts: Arc<KnownHosts>,
    pub(crate) audit: Arc<AuditLog>,
    pub(crate) terminals: Arc<TerminalManager>,
    pub(crate) sftp_sessions: Mutex<HashMap<HostId, Arc<dyn SftpClient>>>,
    pub(crate) file_plans: FilePlanStore,
    pub(crate) transfers: TransferManager,
    sessions: Mutex<HashMap<HostId, Arc<SessionSlot>>>,
    mutation: Mutex<()>,
    _profile_lock: Option<File>,
}
fn persistence() -> AppError {
    AppError::new(
        ErrorCode::Persistence,
        "Cannot open the NexusOps data directory.",
    )
}
fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Connection was cancelled.")
}
fn error_outcome(error: &AppError) -> AuditOutcome {
    if error.code == ErrorCode::Cancelled {
        AuditOutcome::Cancelled
    } else {
        AuditOutcome::Failed
    }
}
mod connection;
mod files;
mod hosts;
mod identity;
mod lifecycle;
mod terminal;
#[cfg(test)]
mod tests;
impl Application {
    pub fn open(directory: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(directory).map_err(|_| persistence())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| persistence())?;
        }
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("profile.lock"))
            .map_err(|_| persistence())?;
        lock.try_lock().map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "This NexusOps profile is already open in another process.",
            )
        })?;
        let repository = Arc::new(HostRepository::open(directory.join("hosts.db"))?);
        let keys = Arc::new(PlatformKeyProvider::new(&repository.profile_id()?));
        let secrets = Arc::new(EncryptedSecretStore::open(
            directory.join("credentials.db"),
            keys,
        )?);
        let known_hosts = Arc::new(KnownHosts::open(directory.join("known-hosts.db"))?);
        let audit = Arc::new(AuditLog::open(directory.join("audit.jsonl"))?);
        let provider = Arc::new(SshProvider::new(known_hosts.clone()));
        Ok(Self {
            repository,
            secrets,
            provider,
            known_hosts,
            audit,
            terminals: Arc::new(TerminalManager::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
            file_plans: FilePlanStore::default(),
            transfers: TransferManager::new(),
            sessions: Mutex::new(HashMap::new()),
            mutation: Mutex::new(()),
            _profile_lock: Some(lock),
        })
    }
    async fn slot(&self, id: HostId) -> Arc<SessionSlot> {
        self.sessions
            .lock()
            .await
            .entry(id)
            .or_insert_with(|| Arc::new(SessionSlot::new(id)))
            .clone()
    }
    fn record(
        &self,
        id: HostId,
        kind: &str,
        outcome: AuditOutcome,
        started: Instant,
    ) -> Result<(), AppError> {
        self.audit.record(&AuditEvent::new(
            id,
            Operation {
                id: HostId::new().to_string(),
                kind: kind.into(),
                risk: match kind {
                    "identity.trust" => OperationRisk::High,
                    "host.save" | "host.delete" => OperationRisk::Low,
                    "file.create_directory" => OperationRisk::Low,
                    "file.rename" | "file.transfer.accepted" => OperationRisk::Moderate,
                    "file.delete" => OperationRisk::Destructive,
                    _ => OperationRisk::ReadOnly,
                },
            },
            AuditActor::User,
            outcome,
            started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        ))
    }
    /// Cancels sockets immediately during desktop shutdown; no detached SSH lifetime survives.
    pub async fn shutdown(&self) {
        self.transfers.shutdown();
        self.terminals.shutdown().await;
        let sftp = std::mem::take(&mut *self.sftp_sessions.lock().await);
        for session in sftp.into_values() {
            session.close().await;
        }
        for slot in self.sessions.lock().await.values() {
            let mut data = slot.data.lock().await;
            data.generation += 1;
            data.cancel.cancel();
            data.refreshing = false;
            data.transport = None;
            data.connection_id = None;
            data.view = HostSession::disconnected(data.view.host_id);
        }
    }
}
