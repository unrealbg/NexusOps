use super::*;
use crate::ConnectedTransport;
use async_trait::async_trait;
use nexus_operations::{ReadOnlyCommand, RemoteSession};
use nexus_secrets::{Credential, KeyProvider};
use nexus_terminal::{TerminalChannel, TerminalConnector, TerminalRead};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;
use zeroize::Zeroizing;

struct TestKey;
impl KeyProvider for TestKey {
    fn key(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        Ok(Zeroizing::new(vec![42; 32]))
    }
}
struct TestProvider {
    stall: AtomicBool,
    started: Notify,
    release: Notify,
}
#[async_trait]
impl ConnectionProvider for TestProvider {
    async fn connect(
        &self,
        _: &Host,
        _: Credential,
        cancel: CancellationToken,
    ) -> Result<Arc<dyn ConnectedTransport>, AppError> {
        if self.stall.swap(false, Ordering::SeqCst) {
            self.started.notify_one();
            // Intentionally ignore cancellation to exercise the application generation guard.
            self.release.notified().await;
        }
        Ok(Arc::new(TestSession { cancel }))
    }
}
struct TestSession {
    cancel: CancellationToken,
}
#[async_trait]
impl RemoteSession for TestSession {
    async fn execute(
        &self,
        command: ReadOnlyCommand,
        _: CancellationToken,
    ) -> Result<String, AppError> {
        Ok(match command {
            ReadOnlyCommand::Hostname=>"test-linux\n",
            ReadOnlyCommand::OsRelease=>"NAME=Linux\nVERSION_ID=1\n",
            ReadOnlyCommand::Kernel=>"6.8.0\n",
            ReadOnlyCommand::Architecture=>"x86_64\n",
            ReadOnlyCommand::Uptime=>"3600.0 1.0\n",
            ReadOnlyCommand::LoadAverage=>"0.1 0.2 0.3 1/100 1\n",
            ReadOnlyCommand::Memory=>"MemTotal: 8192 kB\nMemAvailable: 4096 kB\n",
            ReadOnlyCommand::RootFilesystem=>"Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 8192 4096 4096 50% /\n",
        }.into())
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        self.cancel.cancel();
        Ok(())
    }
    fn is_closed(&self) -> bool {
        self.cancel.is_cancelled()
    }
}
#[async_trait]
impl TerminalConnector for TestSession {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        cancellation: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        if self.is_closed() {
            return Err(AppError::new(
                ErrorCode::TerminalUnavailable,
                "Test transport closed.",
            ));
        }
        Ok(Arc::new(TestTerminalChannel {
            host_cancel: self.cancel.clone(),
            terminal_cancel: cancellation,
        }))
    }
}
struct TestTerminalChannel {
    host_cancel: CancellationToken,
    terminal_cancel: CancellationToken,
}
#[async_trait]
impl TerminalChannel for TestTerminalChannel {
    async fn read(&self) -> Result<TerminalRead, AppError> {
        tokio::select! {
            _ = self.host_cancel.cancelled() => {},
            _ = self.terminal_cancel.cancelled() => {},
        }
        Err(AppError::new(
            ErrorCode::Connection,
            "Test terminal disconnected.",
        ))
    }
    async fn write(&self, _: &[u8]) -> Result<(), AppError> {
        Ok(())
    }
    async fn resize(&self, _: TerminalSize) -> Result<(), AppError> {
        Ok(())
    }
    async fn close(&self) -> Result<(), AppError> {
        self.terminal_cancel.cancel();
        Ok(())
    }
}
fn setup(stall: bool) -> (tempfile::TempDir, Arc<Application>, Arc<TestProvider>) {
    let dir = tempfile::tempdir().expect("dir");
    let provider = Arc::new(TestProvider {
        stall: AtomicBool::new(stall),
        started: Notify::new(),
        release: Notify::new(),
    });
    let application = Application {
        repository: Arc::new(HostRepository::open(dir.path().join("hosts.db")).expect("hosts")),
        secrets: Arc::new(
            EncryptedSecretStore::open(dir.path().join("secrets.db"), Arc::new(TestKey))
                .expect("secrets"),
        ),
        known_hosts: Arc::new(KnownHosts::open(dir.path().join("pins.db")).expect("pins")),
        provider: provider.clone(),
        audit: Arc::new(AuditLog::open(dir.path().join("audit.jsonl")).expect("audit")),
        terminals: Arc::new(nexus_terminal::TerminalManager::new()),
        mutation: Mutex::new(()),
        sessions: Mutex::new(HashMap::new()),
        _profile_lock: None,
    };
    (dir, Arc::new(application), provider)
}
fn input() -> HostInput {
    HostInput {
        id: None,
        display_name: "Test host".into(),
        connection: HostConnectionConfig {
            hostname: "localhost".into(),
            port: 22,
            username: "user".into(),
            authentication: AuthenticationMethod::Password,
        },
    }
}
fn credential() -> CredentialInput {
    CredentialInput {
        password: Some("test-only-secret".into()),
        private_key: None,
        passphrase: None,
    }
}

fn terminal_size() -> TerminalSize {
    TerminalSize {
        columns: 100,
        rows: 30,
        pixel_width: 900,
        pixel_height: 540,
    }
}

#[test]
fn a_profile_cannot_open_twice_and_releases_its_lock_on_drop() {
    let directory = tempfile::tempdir().expect("directory");
    let first = Application::open(directory.path()).expect("open profile");
    assert_eq!(
        Application::open(directory.path())
            .err()
            .expect("duplicate profile")
            .code,
        ErrorCode::Conflict
    );
    drop(first);
    assert!(Application::open(directory.path()).is_ok());
}

#[tokio::test]
async fn host_crud_revisions_and_connected_edit_policy() {
    let (_dir, app, _) = setup(false);
    assert_eq!(
        app.save_host(input(), None)
            .await
            .expect_err("missing credential")
            .code,
        ErrorCode::Validation
    );
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("create");
    let old_ref = app.repository.get(host.id).expect("host").credential_id;
    let mut edit = input();
    edit.id = Some(host.id);
    edit.display_name = "Updated".into();
    app.save_host(edit.clone(), Some(credential()))
        .await
        .expect("update");
    assert!(app.secrets.get(old_ref).is_err());
    app.connect_host(host.id).await.expect("connect");
    assert_eq!(
        app.get_session(host.id).await.expect("session").state,
        ConnectionState::Connected
    );
    assert_eq!(
        app.save_host(edit, None)
            .await
            .expect_err("connected edit")
            .code,
        ErrorCode::Conflict
    );
    app.delete_host(host.id).await.expect("delete");
    assert!(app.list_hosts().expect("hosts").is_empty());
}

#[tokio::test]
async fn cancellation_cannot_publish_stale_results_and_hosts_are_independent() {
    let (dir, app, provider) = setup(true);
    let first = app
        .save_host(input(), Some(credential()))
        .await
        .expect("first");
    let second = app
        .save_host(input(), Some(credential()))
        .await
        .expect("second");
    let task_app = app.clone();
    let task = tokio::spawn(async move { task_app.connect_host(first.id).await });
    provider.started.notified().await;
    assert_eq!(
        app.get_session(first.id).await.expect("first").state,
        ConnectionState::Connecting
    );
    app.connect_host(second.id).await.expect("second connected");
    app.disconnect_host(first.id).await.expect("cancel first");
    provider.release.notify_one();
    assert_eq!(
        task.await.expect("task").expect_err("cancelled").code,
        ErrorCode::Cancelled
    );
    assert!(
        audit_events(dir.path())
            .iter()
            .any(|event| event.host == first.id
                && event.operation.kind == "connection.connect"
                && event.outcome == AuditOutcome::Cancelled)
    );
    assert_eq!(
        app.get_session(first.id).await.expect("first").state,
        ConnectionState::Disconnected
    );
    assert_eq!(
        app.get_session(second.id).await.expect("second").state,
        ConnectionState::Connected
    );
    assert!(
        app.get_session(first.id)
            .await
            .expect("first")
            .discovery
            .is_none()
    );
    app.reconnect_host(first.id).await.expect("reconnect");
    assert!(
        app.get_session(first.id)
            .await
            .expect("reconnected")
            .discovery
            .is_some()
    );
}

#[tokio::test]
async fn repeated_connect_and_shutdown_leave_no_live_or_stale_session() {
    let (_dir, app, provider) = setup(true);
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("host");
    let task_app = app.clone();
    let task = tokio::spawn(async move { task_app.connect_host(host.id).await });
    provider.started.notified().await;
    assert_eq!(
        app.connect_host(host.id)
            .await
            .expect_err("repeat connect")
            .code,
        ErrorCode::Conflict
    );
    app.shutdown().await;
    assert_eq!(
        app.get_session(host.id).await.expect("session").state,
        ConnectionState::Disconnected
    );
    provider.release.notify_one();
    assert_eq!(
        task.await
            .expect("connect task")
            .expect_err("cancelled")
            .code,
        ErrorCode::Cancelled
    );
    let session = app.get_session(host.id).await.expect("final session");
    assert_eq!(session.state, ConnectionState::Disconnected);
    assert!(session.discovery.is_none());

    let (_dir, connected_app, _) = setup(false);
    let connected_host = connected_app
        .save_host(input(), Some(credential()))
        .await
        .expect("connected host");
    connected_app
        .connect_host(connected_host.id)
        .await
        .expect("connect");
    connected_app.shutdown().await;
    assert_eq!(
        connected_app
            .get_session(connected_host.id)
            .await
            .expect("shutdown session")
            .state,
        ConnectionState::Disconnected
    );
}

#[tokio::test]
async fn trust_requires_exact_current_challenge_and_is_cleared_by_edit() {
    let (_dir, app, _) = setup(false);
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("host");
    let challenge = HostKeyChallenge {
        hostname: "localhost".into(),
        port: 22,
        algorithm: "ssh-ed25519".into(),
        fingerprint: format!("SHA256:{}", "A".repeat(43)),
        previous_fingerprint: None,
    };
    assert_eq!(
        app.trust_host_key(host.id, challenge.clone())
            .await
            .expect_err("not pending")
            .code,
        ErrorCode::Conflict
    );
    let slot = app.slot(host.id).await;
    {
        let mut data = slot.data.lock().await;
        data.view.state = ConnectionState::AwaitingTrust;
        data.view.error = Some(AppError {
            code: ErrorCode::UnknownHostKey,
            message: "Verify server".into(),
            host_key: Some(Box::new(challenge.clone())),
        });
    }
    let mut forged = challenge.clone();
    forged.port = 2222;
    assert!(app.trust_host_key(host.id, forged).await.is_err());
    let mut edit = input();
    edit.id = Some(host.id);
    edit.connection.hostname = "other-host".into();
    app.save_host(edit, None).await.expect("edit");
    assert!(app.trust_host_key(host.id, challenge).await.is_err());
    assert!(
        app.known_hosts
            .fingerprint("localhost", 22)
            .expect("pins")
            .is_none()
    );
}

#[tokio::test]
async fn locked_secure_store_does_not_create_metadata() {
    struct Locked;
    impl SecretStore for Locked {
        fn put(&self, _: HostId, _: &Credential) -> Result<(), AppError> {
            Err(AppError::new(ErrorCode::SecureStorage, "Locked"))
        }
        fn get(&self, _: HostId) -> Result<Credential, AppError> {
            Err(AppError::new(ErrorCode::SecureStorage, "Locked"))
        }
        fn delete(&self, _: HostId) -> Result<(), AppError> {
            Ok(())
        }
    }
    let (_dir, app, _) = setup(false);
    let mut app = Arc::try_unwrap(app).ok().expect("owned application");
    app.secrets = Arc::new(Locked);
    assert_eq!(
        app.save_host(input(), Some(credential()))
            .await
            .expect_err("locked")
            .code,
        ErrorCode::SecureStorage
    );
    assert!(app.list_hosts().expect("list").is_empty());
}

fn audit_events(directory: &Path) -> Vec<AuditEvent> {
    std::fs::read_to_string(directory.join("audit.jsonl"))
        .expect("audit log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("structured event"))
        .collect()
}

#[tokio::test]
async fn authentication_switch_requires_matching_new_credentials_and_preserves_revisions() {
    let (_dir, app, _) = setup(false);
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("create");
    let first = app.repository.get(host.id).expect("stored");
    let mut edit = input();
    edit.id = Some(host.id);
    edit.display_name = "Metadata-only edit".into();
    app.save_host(edit.clone(), None)
        .await
        .expect("retain credentials");
    assert_eq!(
        app.repository.get(host.id).expect("stored").credential_id,
        first.credential_id
    );
    edit.connection.authentication = AuthenticationMethod::PrivateKey;
    assert_eq!(
        app.save_host(edit.clone(), None)
            .await
            .expect_err("new auth needs new secret")
            .code,
        ErrorCode::Validation
    );
    assert_eq!(
        app.save_host(edit.clone(), Some(credential()))
            .await
            .expect_err("password does not match key auth")
            .code,
        ErrorCode::Validation
    );
    assert_eq!(
        app.repository
            .get(host.id)
            .expect("unchanged method")
            .host
            .connection
            .authentication,
        AuthenticationMethod::Password
    );
    assert!(app.secrets.get(first.credential_id).is_ok());
    app.save_host(
        edit,
        Some(CredentialInput {
            password: None,
            private_key: Some("test-only-private-key-material".into()),
            passphrase: Some("test-only-passphrase".into()),
        }),
    )
    .await
    .expect("switch method");
    let updated = app.repository.get(host.id).expect("updated");
    assert_ne!(updated.credential_id, first.credential_id);
    assert!(app.secrets.get(first.credential_id).is_err());
    let loaded = app
        .secrets
        .get(updated.credential_id)
        .expect("new credentials");
    assert!(loaded.password.is_none());
    assert_eq!(
        loaded.private_key.as_ref().map(|value| value.as_str()),
        Some("test-only-private-key-material")
    );
}

struct SlowRefreshSession {
    closed: AtomicBool,
    started: Notify,
    release: Notify,
}
#[async_trait]
impl RemoteSession for SlowRefreshSession {
    async fn execute(&self, _: ReadOnlyCommand, _: CancellationToken) -> Result<String, AppError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok("stale-hostname".into())
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}
#[async_trait]
impl TerminalConnector for SlowRefreshSession {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        _: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        Err(AppError::new(
            ErrorCode::TerminalUnavailable,
            "Test terminal unavailable.",
        ))
    }
}

#[tokio::test]
async fn remote_closure_during_refresh_cannot_publish_stale_metrics_and_is_audited() {
    let (dir, app, _) = setup(false);
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("host");
    app.connect_host(host.id).await.expect("connect");
    let transport = Arc::new(SlowRefreshSession {
        closed: AtomicBool::new(false),
        started: Notify::new(),
        release: Notify::new(),
    });
    app.slot(host.id).await.data.lock().await.transport = Some(transport.clone());
    let task_app = app.clone();
    let refresh = tokio::spawn(async move { task_app.refresh_host(host.id).await });
    transport.started.notified().await;
    assert_eq!(
        app.refresh_host(host.id)
            .await
            .expect_err("concurrent refresh")
            .code,
        ErrorCode::Conflict
    );
    transport.closed.store(true, Ordering::SeqCst);
    let observed = app.get_session(host.id).await.expect("closure observed");
    assert_eq!(observed.state, ConnectionState::Failed);
    assert!(observed.discovery.is_none());
    assert!(observed.capabilities.is_empty());
    transport.release.notify_one();
    assert_eq!(
        refresh
            .await
            .expect("refresh task")
            .expect_err("stale generation")
            .code,
        ErrorCode::Cancelled
    );
    let observed = app.get_session(host.id).await.expect("final state");
    assert_eq!(observed.state, ConnectionState::Failed);
    assert!(observed.discovery.is_none());
    assert!(!app.slot(host.id).await.data.lock().await.refreshing);
    assert!(
        audit_events(dir.path())
            .iter()
            .any(|event| event.operation.kind == "discovery.refresh"
                && event.outcome == AuditOutcome::Cancelled)
    );
}

#[tokio::test]
async fn committed_host_edit_invalidates_trust_even_when_old_secret_cleanup_fails() {
    struct FailingDelete(Arc<dyn SecretStore>);
    impl SecretStore for FailingDelete {
        fn put(&self, id: HostId, value: &Credential) -> Result<(), AppError> {
            self.0.put(id, value)
        }
        fn get(&self, id: HostId) -> Result<Credential, AppError> {
            self.0.get(id)
        }
        fn delete(&self, _: HostId) -> Result<(), AppError> {
            Err(AppError::new(
                ErrorCode::SecureStorage,
                "Test cleanup failure",
            ))
        }
    }
    let (_dir, app, _) = setup(false);
    let mut app = Arc::try_unwrap(app).ok().expect("owned application");
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("host");
    let challenge = HostKeyChallenge {
        hostname: "localhost".into(),
        port: 22,
        algorithm: "ssh-ed25519".into(),
        fingerprint: format!("SHA256:{}", "A".repeat(43)),
        previous_fingerprint: None,
    };
    let slot = app.slot(host.id).await;
    {
        let mut data = slot.data.lock().await;
        data.view.state = ConnectionState::AwaitingTrust;
        data.view.error = Some(AppError {
            code: ErrorCode::UnknownHostKey,
            message: "Trust required".into(),
            host_key: Some(Box::new(challenge.clone())),
        });
    }
    app.secrets = Arc::new(FailingDelete(app.secrets.clone()));
    let mut edit = input();
    edit.id = Some(host.id);
    edit.connection.hostname = "new-endpoint".into();
    assert_eq!(
        app.save_host(edit, Some(credential()))
            .await
            .expect_err("cleanup surfaced")
            .code,
        ErrorCode::SecureStorage
    );
    let stored = app.repository.get(host.id).expect("committed metadata");
    assert_eq!(stored.host.connection.hostname, "new-endpoint");
    assert!(app.secrets.get(stored.credential_id).is_ok());
    assert_eq!(
        app.get_session(host.id).await.expect("session").state,
        ConnectionState::Disconnected
    );
    assert_eq!(
        app.trust_host_key(host.id, challenge)
            .await
            .expect_err("old challenge invalidated")
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn terminals_bind_to_one_connection_and_reconnect_never_reuses_them() {
    let (_dir, app, _) = setup(false);
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("host");
    app.connect_host(host.id).await.expect("connect");
    let first = app
        .open_terminal(host.id, terminal_size())
        .await
        .expect("first terminal");
    assert_eq!(first.state, TerminalState::Open);

    app.disconnect_host(host.id).await.expect("disconnect");
    let ended = app.list_terminals(host.id).await.expect("ended terminals");
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].state, TerminalState::Disconnected);

    app.connect_host(host.id).await.expect("reconnect");
    let second = app
        .open_terminal(host.id, terminal_size())
        .await
        .expect("new terminal");
    assert_ne!(first.host_session_id, second.host_session_id);
    assert_ne!(first.id, second.id);
    assert_eq!(
        app.resize_terminal(host.id, first.host_session_id, first.id, terminal_size(),)
            .await
            .expect_err("old PTY cannot attach to new connection")
            .code,
        ErrorCode::TerminalUnavailable
    );
    assert_eq!(
        app.list_terminals(host.id).await.expect("sessions").len(),
        2
    );
}
