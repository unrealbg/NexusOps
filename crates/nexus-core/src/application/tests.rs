use super::*;
use crate::ConnectedTransport;
use async_trait::async_trait;
use nexus_operations::{ReadOnlyCommand, RemoteSession};
use nexus_secrets::{Credential, KeyProvider};
use nexus_terminal::{TerminalChannel, TerminalConnector, TerminalRead};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncRead, AsyncWrite};
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
    monitor: Arc<MonitorControl>,
    services: Arc<ServiceControl>,
}
struct MonitorControl {
    stall: AtomicBool,
    started: Notify,
}
struct ServiceControl {
    stall: AtomicBool,
    entered: Semaphore,
    release: Semaphore,
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
        Ok(Arc::new(TestSession {
            cancel,
            monitor: self.monitor.clone(),
            services: self.services.clone(),
            monitor_counter: AtomicUsize::new(0),
        }))
    }
}
struct TestSession {
    cancel: CancellationToken,
    monitor: Arc<MonitorControl>,
    services: Arc<ServiceControl>,
    monitor_counter: AtomicUsize,
}
#[async_trait]
impl RemoteSession for TestSession {
    async fn execute(
        &self,
        command: ReadOnlyCommand,
        _: CancellationToken,
    ) -> Result<String, AppError> {
        if command == ReadOnlyCommand::CpuStat && self.monitor.stall.swap(false, Ordering::SeqCst) {
            self.monitor.started.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(match command {
            ReadOnlyCommand::Hostname=>"test-linux\n",
            ReadOnlyCommand::OsRelease=>"NAME=Linux\nVERSION_ID=1\n",
            ReadOnlyCommand::Kernel=>"6.8.0\n",
            ReadOnlyCommand::Architecture=>"x86_64\n",
            ReadOnlyCommand::Uptime=>"3600.0 1.0\n",
            ReadOnlyCommand::LoadAverage=>"0.1 0.2 0.3 1/100 1\n",
            ReadOnlyCommand::Memory=>"MemTotal: 8192 kB\nMemAvailable: 4096 kB\nSwapTotal: 1024 kB\nSwapFree: 768 kB\n",
            ReadOnlyCommand::RootFilesystem=>"Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 8192 4096 4096 50% /\n",
            ReadOnlyCommand::CpuStat => {
                let sample = self.monitor_counter.fetch_add(1, Ordering::SeqCst) as u64;
                return Ok(format!("cpu {} 0 50 {} 10 0 0 0\n", 100 + sample * 50, 850 + sample * 50));
            }
            ReadOnlyCommand::NetworkDevices => {
                let sample = self.monitor_counter.load(Ordering::SeqCst).saturating_sub(1) as u64;
                return Ok(format!("Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n lo: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n eth0: {} 0 0 0 0 0 0 0 {} 0 0 0 0 0 0 0\n", 1000 + sample * 500, 2000 + sample * 1000));
            }
            ReadOnlyCommand::SystemServices => {
                if self.services.stall.load(Ordering::SeqCst) {
                    self.services.entered.add_permits(1);
                    let permit = self.services.release.acquire().await.expect("release");
                    permit.forget();
                }
                "sshd.service loaded active running OpenSSH daemon\n"
            },
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
#[async_trait]
impl nexus_sftp::SftpConnector for TestSession {
    async fn open_sftp(
        &self,
        _: HostId,
        _: HostSessionId,
    ) -> Result<Arc<dyn nexus_sftp::SftpClient>, AppError> {
        Err(AppError::new(
            ErrorCode::SftpUnavailable,
            "SFTP is unavailable in this test provider.",
        ))
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
        monitor: Arc::new(MonitorControl {
            stall: AtomicBool::new(false),
            started: Notify::new(),
        }),
        services: Arc::new(ServiceControl {
            stall: AtomicBool::new(false),
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        }),
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
        sftp_sessions: Mutex::new(HashMap::new()),
        sftp_startups: Mutex::new(HashMap::new()),
        file_plans: nexus_sftp::FilePlanStore::default(),
        transfers: nexus_sftp::TransferManager::new(),
        monitor_baselines: Mutex::new(HashMap::new()),
        monitor_gates: Mutex::new(HashMap::new()),
        service_gates: Mutex::new(HashMap::new()),
        service_limit: Semaphore::new(4),
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
async fn monitoring_is_owned_by_exact_session_and_isolated_by_host() {
    let (_dir, app, _) = setup(false);
    let first = app
        .save_host(input(), Some(credential()))
        .await
        .expect("first");
    let second = app
        .save_host(input(), Some(credential()))
        .await
        .expect("second");
    app.connect_host(first.id).await.expect("connect first");
    app.connect_host(second.id).await.expect("connect second");
    let first_session = app
        .get_session(first.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    let second_session = app
        .get_session(second.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();

    assert_eq!(
        app.sample_host_monitor(first.id, second_session)
            .await
            .expect_err("foreign session")
            .code,
        ErrorCode::Conflict
    );
    let first_sample = app
        .sample_host_monitor(first.id, first_session)
        .await
        .unwrap();
    let second_sample = app
        .sample_host_monitor(second.id, second_session)
        .await
        .unwrap();
    assert!(first_sample.cpu_usage_percent.is_none());
    assert!(second_sample.cpu_usage_percent.is_none());
    assert_eq!(app.monitor_baselines.lock().await.len(), 2);
    let next = app
        .sample_host_monitor(first.id, first_session)
        .await
        .unwrap();
    assert_eq!(next.cpu_usage_percent, Some(50.0));
}

#[tokio::test]
async fn services_require_exact_session_and_keep_hosts_independent() {
    let (_dir, app, _) = setup(false);
    let first = app.save_host(input(), Some(credential())).await.unwrap();
    let second = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(first.id).await.unwrap();
    app.connect_host(second.id).await.unwrap();
    let first_session = app
        .get_session(first.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    let second_session = app
        .get_session(second.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_eq!(
        app.list_host_services(first.id, second_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let first_snapshot = app
        .list_host_services(first.id, first_session)
        .await
        .unwrap();
    let second_snapshot = app
        .list_host_services(second.id, second_session)
        .await
        .unwrap();
    assert_eq!(first_snapshot.host_session_id, first_session);
    assert_eq!(second_snapshot.host_id, second.id);
    assert_eq!(first_snapshot.entries[0].unit, "sshd.service");
    assert_eq!(app.service_gates.lock().await.len(), 2);
    app.delete_host(first.id).await.unwrap();
    assert!(!app.service_gates.lock().await.contains_key(&first.id));
    assert!(
        app.list_host_services(second.id, second_session)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn services_reject_busy_and_delayed_old_session_after_disconnect_reconnect() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let old_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    provider.services.stall.store(true, Ordering::SeqCst);
    let request_app = app.clone();
    let task =
        tokio::spawn(async move { request_app.list_host_services(host.id, old_session).await });
    provider.services.entered.acquire().await.unwrap().forget();
    assert_eq!(
        app.list_host_services(host.id, old_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.disconnect_host(host.id).await.unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Cancelled);
    provider.services.stall.store(false, Ordering::SeqCst);
    app.connect_host(host.id).await.unwrap();
    let new_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_ne!(old_session, new_session);
    assert_eq!(
        app.list_host_services(host.id, old_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(app.list_host_services(host.id, new_session).await.is_ok());
}

#[tokio::test]
async fn services_global_limit_is_non_queuing_and_shutdown_clears_gates() {
    let (_dir, app, provider) = setup(false);
    let mut hosts = Vec::new();
    for _ in 0..5 {
        let host = app.save_host(input(), Some(credential())).await.unwrap();
        app.connect_host(host.id).await.unwrap();
        let session = app
            .get_session(host.id)
            .await
            .unwrap()
            .host_session_id
            .unwrap();
        hosts.push((host.id, session));
    }
    provider.services.stall.store(true, Ordering::SeqCst);
    let tasks = hosts[..4]
        .iter()
        .map(|(host, session)| {
            let app = app.clone();
            let (host, session) = (*host, *session);
            tokio::spawn(async move { app.list_host_services(host, session).await })
        })
        .collect::<Vec<_>>();
    for _ in 0..4 {
        provider.services.entered.acquire().await.unwrap().forget();
    }
    assert_eq!(
        app.list_host_services(hosts[4].0, hosts[4].1)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    provider.services.release.add_permits(4);
    for task in tasks {
        assert!(task.await.unwrap().is_ok());
    }
    app.shutdown().await.unwrap();
    assert!(app.service_gates.lock().await.is_empty());
    assert_eq!(
        app.list_host_services(hosts[0].0, hosts[0].1)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn services_remote_closure_and_host_edit_cannot_restore_old_inventory() {
    let (_dir, app, provider) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let first_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    provider.services.stall.store(true, Ordering::SeqCst);
    let request_app = app.clone();
    let request =
        tokio::spawn(async move { request_app.list_host_services(host.id, first_session).await });
    provider.services.entered.acquire().await.unwrap().forget();
    app.slot(host.id).await.data.lock().await.cancel.cancel();
    assert_eq!(
        app.get_session(host.id).await.unwrap().state,
        ConnectionState::Failed
    );
    assert_eq!(
        request.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        app.list_host_services(host.id, first_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );

    provider.services.stall.store(false, Ordering::SeqCst);
    app.disconnect_host(host.id).await.unwrap();
    let mut edited = input();
    edited.id = Some(host.id);
    edited.display_name = "Renamed host".into();
    app.save_host(edited, None).await.unwrap();
    assert_eq!(
        app.list_host_services(host.id, first_session)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    app.connect_host(host.id).await.unwrap();
    let second_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_ne!(first_session, second_session);
    assert!(
        app.list_host_services(host.id, second_session)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn disconnect_cancels_delayed_monitoring_and_reconnect_resets_baseline() {
    let (_dir, app, provider) = setup(false);
    let host = app
        .save_host(input(), Some(credential()))
        .await
        .expect("host");
    app.connect_host(host.id).await.expect("connect");
    let old_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    app.sample_host_monitor(host.id, old_session).await.unwrap();
    provider.monitor.stall.store(true, Ordering::SeqCst);
    let sample_app = app.clone();
    let task =
        tokio::spawn(async move { sample_app.sample_host_monitor(host.id, old_session).await });
    provider.monitor.started.notified().await;
    app.disconnect_host(host.id).await.expect("disconnect");
    assert_eq!(
        task.await.unwrap().expect_err("cancelled sample").code,
        ErrorCode::Cancelled
    );
    assert!(!app.monitor_baselines.lock().await.contains_key(&host.id));

    app.connect_host(host.id).await.expect("reconnect");
    let new_session = app
        .get_session(host.id)
        .await
        .unwrap()
        .host_session_id
        .unwrap();
    assert_ne!(old_session, new_session);
    assert_eq!(
        app.sample_host_monitor(host.id, old_session)
            .await
            .expect_err("old ownership")
            .code,
        ErrorCode::Conflict
    );
    assert!(
        app.sample_host_monitor(host.id, new_session)
            .await
            .unwrap()
            .cpu_usage_percent
            .is_none()
    );
    app.delete_host(host.id).await.expect("delete");
    assert!(!app.monitor_baselines.lock().await.contains_key(&host.id));
    assert!(!app.monitor_gates.lock().await.contains_key(&host.id));
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
    app.shutdown().await.unwrap();
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
    let connection_id = connected_app
        .get_session(connected_host.id)
        .await
        .expect("connected session")
        .host_session_id
        .expect("connection identity");
    connected_app
        .sample_host_monitor(connected_host.id, connection_id)
        .await
        .expect("monitor baseline");
    connected_app.shutdown().await.unwrap();
    assert!(connected_app.monitor_baselines.lock().await.is_empty());
    assert!(connected_app.monitor_gates.lock().await.is_empty());
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
#[async_trait]
impl nexus_sftp::SftpConnector for SlowRefreshSession {
    async fn open_sftp(
        &self,
        _: HostId,
        _: HostSessionId,
    ) -> Result<Arc<dyn nexus_sftp::SftpClient>, AppError> {
        Err(AppError::new(
            ErrorCode::SftpUnavailable,
            "SFTP is unavailable in this test provider.",
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

struct LifecycleSftpClient {
    info: SftpSessionInfo,
    closes: AtomicUsize,
    identity_calls: AtomicUsize,
    stall_plan_identity: AtomicBool,
    stall_plan_stat: AtomicBool,
    stall_editor_read: AtomicBool,
    plan_remote_started: Notify,
    plan_remote_release: Notify,
    closed: Notify,
    upload_failures: std::sync::Mutex<std::collections::VecDeque<nexus_sftp::StagedUploadFailure>>,
    stall_owned_upload: AtomicBool,
    upload_started: Notify,
    cleanup_started: Notify,
    cleanup_release: Notify,
    cleanup_calls: AtomicUsize,
    cleanup_after_close: AtomicBool,
    fail_cleanup: AtomicBool,
    complete_upload: AtomicBool,
    commit_started: Notify,
    commit_release: Notify,
}

impl LifecycleSftpClient {
    fn new(host_id: HostId, host_session_id: HostSessionId) -> Arc<Self> {
        Arc::new(Self {
            info: SftpSessionInfo {
                id: SftpSessionId::new(),
                host_id,
                host_session_id,
                protocol_version: 3,
                extensions: vec![
                    SftpExtension {
                        name: "hardlink@openssh.com".into(),
                        version: "1".into(),
                    },
                    SftpExtension {
                        name: "posix-rename@openssh.com".into(),
                        version: "1".into(),
                    },
                ],
                limits: SftpLimits {
                    max_packet_bytes: None,
                    max_read_bytes: None,
                    max_write_bytes: None,
                    max_open_handles: None,
                    client_chunk_bytes: "65536".into(),
                    listing_entry_cap: 5_000,
                },
                root_path: "/tmp".into(),
            },
            closes: AtomicUsize::new(0),
            identity_calls: AtomicUsize::new(0),
            stall_plan_identity: AtomicBool::new(false),
            stall_plan_stat: AtomicBool::new(false),
            stall_editor_read: AtomicBool::new(false),
            plan_remote_started: Notify::new(),
            plan_remote_release: Notify::new(),
            closed: Notify::new(),
            upload_failures: std::sync::Mutex::new(std::collections::VecDeque::new()),
            stall_owned_upload: AtomicBool::new(false),
            upload_started: Notify::new(),
            cleanup_started: Notify::new(),
            cleanup_release: Notify::new(),
            cleanup_calls: AtomicUsize::new(0),
            cleanup_after_close: AtomicBool::new(false),
            fail_cleanup: AtomicBool::new(false),
            complete_upload: AtomicBool::new(false),
            commit_started: Notify::new(),
            commit_release: Notify::new(),
        })
    }
}

fn unused_sftp() -> AppError {
    AppError::new(ErrorCode::SftpProtocol, "Unused lifecycle-test operation.")
}

#[async_trait]
impl nexus_sftp::SftpClient for LifecycleSftpClient {
    fn info(&self) -> SftpSessionInfo {
        self.info.clone()
    }
    async fn list(&self, _: &str, _: CancellationToken) -> Result<DirectoryListing, AppError> {
        Err(unused_sftp())
    }
    async fn stat(&self, path: &str) -> Result<RemoteEntry, AppError> {
        if self.stall_plan_stat.swap(false, Ordering::SeqCst) {
            self.plan_remote_started.notify_one();
            self.plan_remote_release.notified().await;
        }
        if path != "/tmp/remote-source.bin" {
            return Err(unused_sftp());
        }
        Ok(RemoteEntry {
            name: "remote-source.bin".into(),
            display_name: "remote-source.bin".into(),
            path: path.into(),
            kind: RemoteEntryKind::File,
            size_bytes: Some("4".into()),
            modified_at: None,
            permissions: None,
            uid: None,
            gid: None,
        })
    }
    async fn identity(&self, path: &str) -> Result<Option<nexus_sftp::EntryIdentity>, AppError> {
        self.identity_calls.fetch_add(1, Ordering::SeqCst);
        if self.stall_plan_identity.swap(false, Ordering::SeqCst) {
            self.plan_remote_started.notify_one();
            self.plan_remote_release.notified().await;
        }
        Ok(
            (path == "/tmp/remote-source.bin").then_some(nexus_sftp::EntryIdentity {
                kind: RemoteEntryKind::File,
                size: Some(4),
                modified: Some(1),
            }),
        )
    }
    async fn editor_revision(&self, path: &str) -> Result<nexus_sftp::EditorRevision, AppError> {
        if path != "/tmp/editor.txt" {
            return Err(unused_sftp());
        }
        Ok(nexus_sftp::EditorRevision {
            identity: nexus_sftp::EntryIdentity {
                kind: RemoteEntryKind::File,
                size: Some(4),
                modified: Some(1),
            },
            raw_mode: 0o100640,
            uid: 1000,
            gid: 1000,
        })
    }
    async fn read_editor_bytes(&self, path: &str) -> Result<Vec<u8>, AppError> {
        if path != "/tmp/editor.txt" {
            return Err(unused_sftp());
        }
        if self.stall_editor_read.swap(false, Ordering::SeqCst) {
            self.plan_remote_started.notify_one();
            self.plan_remote_release.notified().await;
        }
        Ok(b"data".to_vec())
    }
    async fn preserve_editor_metadata(
        &self,
        _: &str,
        _: &nexus_sftp::EditorRevision,
    ) -> Result<(), AppError> {
        Ok(())
    }
    async fn create_dir(&self, _: &str) -> Result<(), AppError> {
        Err(unused_sftp())
    }
    async fn remove_file(&self, _: &str) -> Result<(), AppError> {
        Err(unused_sftp())
    }
    async fn remove_dir(&self, _: &str) -> Result<(), AppError> {
        Err(unused_sftp())
    }
    async fn rename_noclobber(&self, _: &str, _: &str) -> Result<(), AppError> {
        Err(unused_sftp())
    }
    async fn upload_staged(
        &self,
        _: &mut (dyn AsyncRead + Unpin + Send),
        _: &str,
        ownership: nexus_sftp::StagingOwnership,
        cancel: CancellationToken,
        progress: nexus_sftp::Progress,
    ) -> nexus_sftp::StagedUploadResult {
        if self.stall_owned_upload.load(Ordering::SeqCst) {
            ownership.mark_owned();
            progress(8);
            self.upload_started.notify_one();
            cancel.cancelled().await;
            return Err(nexus_sftp::StagedUploadFailure::OwnedFailure(
                AppError::new(ErrorCode::Cancelled, "Upload cancelled."),
            ));
        }
        if self.complete_upload.load(Ordering::SeqCst) {
            ownership.mark_owned();
            progress(8);
            return Ok(8);
        }
        let failure = self
            .upload_failures
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| nexus_sftp::StagedUploadFailure::NotCreated(unused_sftp()));
        if matches!(failure, nexus_sftp::StagedUploadFailure::OwnedFailure(_)) {
            ownership.mark_owned();
        }
        Err(failure)
    }
    async fn download(
        &self,
        _: &str,
        _: &mut (dyn AsyncWrite + Unpin + Send),
        _: CancellationToken,
        _: nexus_sftp::Progress,
    ) -> Result<u64, AppError> {
        Err(unused_sftp())
    }
    async fn commit_new(&self, _: &str, _: &str) -> Result<(), AppError> {
        if self.complete_upload.load(Ordering::SeqCst) {
            self.commit_started.notify_one();
            self.commit_release.notified().await;
            Ok(())
        } else {
            Err(unused_sftp())
        }
    }
    async fn commit_replace(&self, _: &str, _: &str) -> Result<(), AppError> {
        Err(unused_sftp())
    }
    async fn remove_owned_staging(&self, _: &str) -> Result<nexus_sftp::StagingCleanup, AppError> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        if self.closes.load(Ordering::SeqCst) != 0 {
            self.cleanup_after_close.store(true, Ordering::SeqCst);
        }
        self.cleanup_started.notify_one();
        if self.stall_owned_upload.load(Ordering::SeqCst) {
            self.cleanup_release.notified().await;
        }
        if self.fail_cleanup.load(Ordering::SeqCst) {
            Err(AppError::new(
                ErrorCode::SftpDenied,
                "Cleanup REMOVE denied.",
            ))
        } else if self.cleanup_after_close.load(Ordering::SeqCst) {
            Err(AppError::new(
                ErrorCode::Connection,
                "Cleanup ran after SFTP close.",
            ))
        } else {
            Ok(nexus_sftp::StagingCleanup::Removed)
        }
    }
    async fn close(&self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
        self.closed.notify_one();
    }
}

#[tokio::test]
async fn disconnect_cannot_revoke_before_inflight_upload_plan_is_published() {
    let (_profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session_id = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, session_id);
    client.stall_plan_identity.store(true, Ordering::SeqCst);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pending.bin");
    std::fs::write(&path, b"pending upload").unwrap();
    let source =
        nexus_sftp::open_local_source(path.canonicalize().unwrap(), "pending.bin".into()).unwrap();
    let source_handle = Arc::downgrade(&source.handle);
    let planning_app = app.clone();
    let sftp_id = client.info.id;
    let planning = tokio::spawn(async move {
        planning_app
            .plan_upload(
                host.id,
                session_id,
                sftp_id,
                vec![source],
                "/tmp".into(),
                ConflictPolicy::Replace,
            )
            .await
    });
    let plan = finish_plan_before_disconnect(&app, host.id, session_id, &client, planning).await;
    assert!(
        source_handle.upgrade().is_none(),
        "the local source handle survived disconnect"
    );
    app.connect_host(host.id).await.unwrap();
    let new_session = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let new_client = LifecycleSftpClient::new(host.id, new_session);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, new_client.clone());
    assert_ne!(new_session, session_id);
    assert!(
        app.execute_file_plan(host.id, new_session, new_client.info.id, plan.id)
            .await
            .is_err()
    );
    assert!(
        app.execute_file_plan(host.id, session_id, sftp_id, plan.id)
            .await
            .is_err()
    );
}

async fn setup_editor_lifecycle() -> (
    tempfile::TempDir,
    Arc<Application>,
    HostId,
    HostSessionId,
    Arc<LifecycleSftpClient>,
) {
    let (profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let connection = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, connection);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    (profile, app, host.id, connection, client)
}

#[tokio::test]
async fn editor_document_and_save_plan_are_revoked_after_reconnect() {
    let (_profile, app, host, connection, client) = setup_editor_lifecycle().await;
    let first = app
        .open_remote_text_file(host, connection, client.info.id, "/tmp/editor.txt".into())
        .await
        .unwrap();
    let plan = app
        .plan_remote_text_save(
            host,
            connection,
            client.info.id,
            first.id,
            "new-data".into(),
        )
        .await
        .unwrap();
    let still_open = app
        .open_remote_text_file(host, connection, client.info.id, "/tmp/editor.txt".into())
        .await
        .unwrap();
    app.disconnect_host(host).await.unwrap();
    app.connect_host(host).await.unwrap();
    let new_connection = app
        .slot(host)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let new_client = LifecycleSftpClient::new(host, new_connection);
    app.sftp_sessions
        .lock()
        .await
        .insert(host, new_client.clone());
    assert_ne!(connection, new_connection);
    assert!(
        app.plan_remote_text_save(
            host,
            connection,
            client.info.id,
            still_open.id,
            "new-data".into()
        )
        .await
        .is_err()
    );
    assert!(
        app.plan_remote_text_save(
            host,
            new_connection,
            new_client.info.id,
            still_open.id,
            "new-data".into()
        )
        .await
        .is_err()
    );
    assert!(
        app.execute_file_plan(host, connection, client.info.id, plan.id)
            .await
            .is_err()
    );
    assert!(
        app.execute_file_plan(host, new_connection, new_client.info.id, plan.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn editor_planning_cannot_leave_authority_after_teardown() {
    let (_profile, app, host, connection, client) = setup_editor_lifecycle().await;
    let document = app
        .open_remote_text_file(host, connection, client.info.id, "/tmp/editor.txt".into())
        .await
        .unwrap();
    client.stall_editor_read.store(true, Ordering::SeqCst);
    let planning_app = app.clone();
    let sftp = client.info.id;
    let planning = tokio::spawn(async move {
        planning_app
            .plan_remote_text_save(host, connection, sftp, document.id, "new-data".into())
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.plan_remote_started.notified(),
    )
    .await
    .unwrap();
    let disconnect_app = app.clone();
    let disconnecting = tokio::spawn(async move { disconnect_app.disconnect_host(host).await });
    tokio::task::yield_now().await;
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    client.plan_remote_release.notify_one();
    let plan = planning.await.unwrap().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), disconnecting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        app.file_plans
            .consume(plan.id, host, connection, sftp)
            .is_err()
    );
    assert!(
        app.execute_file_plan(host, connection, sftp, plan.id)
            .await
            .is_err()
    );
}

async fn start_stalled_editor_save() -> (
    tempfile::TempDir,
    Arc<Application>,
    HostId,
    Arc<LifecycleSftpClient>,
    TransferJobId,
) {
    let (profile, app, host, connection, client) = setup_editor_lifecycle().await;
    let document = app
        .open_remote_text_file(host, connection, client.info.id, "/tmp/editor.txt".into())
        .await
        .unwrap();
    let plan = app
        .plan_remote_text_save(
            host,
            connection,
            client.info.id,
            document.id,
            "new-data".into(),
        )
        .await
        .unwrap();
    client.stall_owned_upload.store(true, Ordering::SeqCst);
    let job = app
        .execute_file_plan(host, connection, client.info.id, plan.id)
        .await
        .unwrap()[0]
        .id;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.upload_started.notified(),
    )
    .await
    .unwrap();
    (profile, app, host, client, job)
}

#[tokio::test]
async fn editor_save_delete_waits_for_owned_cleanup() {
    let (_profile, app, host, client, job) = start_stalled_editor_save().await;
    let deleting_app = app.clone();
    let deleting = tokio::spawn(async move { deleting_app.delete_host(host).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .unwrap();
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    client.cleanup_release.notify_one();
    deleting.await.unwrap().unwrap();
    assert_eq!(
        wait_for_transfer(&app, job).await.state,
        TransferState::Cancelled
    );
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    assert!(!client.cleanup_after_close.load(Ordering::SeqCst));
}

#[tokio::test]
async fn editor_save_shutdown_waits_for_owned_cleanup() {
    let (_profile, app, host, client, job) = start_stalled_editor_save().await;
    let shutdown_app = app.clone();
    let shutting_down = tokio::spawn(async move { shutdown_app.shutdown().await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .unwrap();
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    client.cleanup_release.notify_one();
    shutting_down.await.unwrap().unwrap();
    assert_eq!(
        wait_for_transfer(&app, job).await.state,
        TransferState::Cancelled
    );
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    assert!(!client.cleanup_after_close.load(Ordering::SeqCst));
    assert!(app.slot(host).await.data.lock().await.cancel.is_cancelled());
}

async fn finish_plan_before_disconnect(
    app: &Arc<Application>,
    host: HostId,
    host_session: HostSessionId,
    client: &Arc<LifecycleSftpClient>,
    planning: tokio::task::JoinHandle<Result<FileOperationPlan, AppError>>,
) -> FileOperationPlan {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.plan_remote_started.notified(),
    )
    .await
    .expect("planning reached remote operation after ownership validation");
    let disconnect_app = app.clone();
    let disconnecting = tokio::spawn(async move { disconnect_app.disconnect_host(host).await });
    let closed_before_publication = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        client.closed.notified(),
    )
    .await
    .is_ok();
    client.plan_remote_release.notify_one();
    let plan = planning.await.unwrap().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), disconnecting)
        .await
        .expect("disconnect completed")
        .unwrap()
        .unwrap();
    let stale_plan_published = app
        .file_plans
        .consume(plan.id, host, host_session, client.info.id)
        .is_ok();
    assert!(
        !stale_plan_published,
        "an old-session plan was published after revocation"
    );
    assert!(
        !closed_before_publication,
        "SFTP closed while remote planning was in flight"
    );
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    plan
}

#[tokio::test]
async fn disconnect_releases_inflight_download_directory_plan() {
    let (_profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session_id = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, session_id);
    client.stall_plan_stat.store(true, Ordering::SeqCst);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let directory = tempfile::tempdir().unwrap();
    let destination =
        nexus_sftp::open_local_directory(directory.path().canonicalize().unwrap()).unwrap();
    let directory_handle = Arc::downgrade(&destination.handle);
    let planning_app = app.clone();
    let sftp_id = client.info.id;
    let planning = tokio::spawn(async move {
        planning_app
            .plan_download(
                host.id,
                session_id,
                sftp_id,
                vec!["/tmp/remote-source.bin".into()],
                destination,
                ConflictPolicy::Replace,
            )
            .await
    });
    finish_plan_before_disconnect(&app, host.id, session_id, &client, planning).await;
    assert!(
        directory_handle.upgrade().is_none(),
        "the local directory handle survived disconnect"
    );
}

#[tokio::test]
async fn disconnect_serializes_all_mutation_plan_publications() {
    for mutation in ["create", "rename", "delete"] {
        let (_profile, app, _) = setup(false);
        let host = app.save_host(input(), Some(credential())).await.unwrap();
        app.connect_host(host.id).await.unwrap();
        let session_id = app
            .slot(host.id)
            .await
            .data
            .lock()
            .await
            .connection_id
            .unwrap();
        let client = LifecycleSftpClient::new(host.id, session_id);
        if mutation == "delete" {
            client.stall_plan_stat.store(true, Ordering::SeqCst);
        } else {
            client.stall_plan_identity.store(true, Ordering::SeqCst);
        }
        app.sftp_sessions
            .lock()
            .await
            .insert(host.id, client.clone());
        let planning_app = app.clone();
        let sftp_id = client.info.id;
        let planning = tokio::spawn(async move {
            match mutation {
                "create" => {
                    planning_app
                        .plan_create_directory(
                            host.id,
                            session_id,
                            sftp_id,
                            "/tmp".into(),
                            "new-directory".into(),
                        )
                        .await
                }
                "rename" => {
                    planning_app
                        .plan_rename(
                            host.id,
                            session_id,
                            sftp_id,
                            "/tmp/remote-source.bin".into(),
                            "renamed.bin".into(),
                        )
                        .await
                }
                "delete" => {
                    planning_app
                        .plan_delete(
                            host.id,
                            session_id,
                            sftp_id,
                            "/tmp/remote-source.bin".into(),
                        )
                        .await
                }
                _ => unreachable!(),
            }
        });
        finish_plan_before_disconnect(&app, host.id, session_id, &client, planning).await;
    }
}

struct LifecycleTransport {
    cancel: CancellationToken,
    opens: AtomicUsize,
    open_started: Notify,
    release_open: Notify,
    clients: std::sync::Mutex<Vec<Arc<LifecycleSftpClient>>>,
}

impl LifecycleTransport {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: CancellationToken::new(),
            opens: AtomicUsize::new(0),
            open_started: Notify::new(),
            release_open: Notify::new(),
            clients: std::sync::Mutex::new(vec![]),
        })
    }
}

#[async_trait]
impl RemoteSession for LifecycleTransport {
    async fn execute(&self, _: ReadOnlyCommand, _: CancellationToken) -> Result<String, AppError> {
        Ok(String::new())
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
impl TerminalConnector for LifecycleTransport {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        _: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        Err(AppError::new(
            ErrorCode::TerminalUnavailable,
            "Unused lifecycle terminal.",
        ))
    }
}

#[async_trait]
impl nexus_sftp::SftpConnector for LifecycleTransport {
    async fn open_sftp(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<Arc<dyn nexus_sftp::SftpClient>, AppError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.open_started.notify_one();
        self.release_open.notified().await;
        let client = LifecycleSftpClient::new(host_id, host_session_id);
        self.clients.lock().unwrap().push(client.clone());
        Ok(client)
    }
}

async fn install_lifecycle_transport(
    app: &Application,
    host_id: HostId,
    transport: Arc<LifecycleTransport>,
) -> HostSessionId {
    let slot = app.slot(host_id).await;
    let mut data = slot.data.lock().await;
    let connection_id = data.connection_id.expect("connected session identity");
    data.transport = Some(transport);
    connection_id
}

#[tokio::test]
async fn concurrent_sftp_open_is_coalesced_for_one_connection() {
    let (_directory, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let transport = LifecycleTransport::new();
    install_lifecycle_transport(&app, host.id, transport.clone()).await;

    let first_app = app.clone();
    let second_app = app.clone();
    let first = tokio::spawn(async move { first_app.open_sftp(host.id).await });
    transport.open_started.notified().await;
    let second = tokio::spawn(async move { second_app.open_sftp(host.id).await });
    tokio::task::yield_now().await;
    assert_eq!(transport.opens.load(Ordering::SeqCst), 1);
    transport.release_open.notify_one();
    let first_info = first.await.unwrap().unwrap();
    let second_info = second.await.unwrap().unwrap();
    assert_eq!(first_info.id, second_info.id);
    assert_eq!(transport.opens.load(Ordering::SeqCst), 1);
    assert_eq!(
        transport.clients.lock().unwrap()[0]
            .closes
            .load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn disconnect_during_sftp_startup_cannot_publish_or_close_a_new_session() {
    let (_directory, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let transport = LifecycleTransport::new();
    let old_connection = install_lifecycle_transport(&app, host.id, transport.clone()).await;

    let open_app = app.clone();
    let opening = tokio::spawn(async move { open_app.open_sftp(host.id).await });
    transport.open_started.notified().await;
    let disconnect_app = app.clone();
    let disconnecting = tokio::spawn(async move { disconnect_app.disconnect_host(host.id).await });
    tokio::task::yield_now().await;
    transport.release_open.notify_one();
    assert_eq!(
        opening.await.unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    disconnecting.await.unwrap().unwrap();
    assert!(app.sftp_sessions.lock().await.get(&host.id).is_none());
    assert_eq!(
        transport.clients.lock().unwrap()[0]
            .closes
            .load(Ordering::SeqCst),
        1
    );

    let new_connection = HostSessionId::new();
    let new_client = LifecycleSftpClient::new(host.id, new_connection);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, new_client.clone());
    app.close_sftp(host.id, old_connection).await.unwrap();
    assert_eq!(
        app.sftp_sessions
            .lock()
            .await
            .get(&host.id)
            .unwrap()
            .info()
            .id,
        new_client.info.id
    );
    assert_eq!(new_client.closes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn active_disconnect_waits_for_owned_staging_cleanup_before_sftp_close() {
    let (_profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session_id = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, session_id);
    client.stall_owned_upload.store(true, Ordering::SeqCst);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("active-upload.bin");
    std::fs::write(&source, b"12345678").unwrap();
    let plan = app
        .plan_upload(
            host.id,
            session_id,
            client.info.id,
            vec![
                nexus_sftp::open_local_source(
                    source.canonicalize().unwrap(),
                    "active-upload.bin".into(),
                )
                .unwrap(),
            ],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, session_id, client.info.id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.upload_started.notified(),
    )
    .await
    .expect("owned upload started");
    assert_eq!(app.list_transfers(Some(host.id))[0].confirmed_bytes, "8");
    let disconnect_app = app.clone();
    let disconnecting = tokio::spawn(async move { disconnect_app.disconnect_host(host.id).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .expect("owned cleanup started");
    assert_eq!(
        client.closes.load(Ordering::SeqCst),
        0,
        "SFTP closed before staging cleanup"
    );
    assert!(
        !app.slot(host.id)
            .await
            .data
            .lock()
            .await
            .cancel
            .is_cancelled(),
        "SSH lifetime ended before staging cleanup"
    );
    client.cleanup_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), disconnecting)
        .await
        .expect("disconnect quiesced")
        .unwrap()
        .unwrap();
    assert_eq!(client.cleanup_calls.load(Ordering::SeqCst), 1);
    assert!(!client.cleanup_after_close.load(Ordering::SeqCst));
    assert_eq!(
        wait_for_transfer(&app, job.id).await.state,
        TransferState::Cancelled
    );
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    #[cfg(windows)]
    std::fs::rename(&source, directory.path().join("source-released.bin")).unwrap();
}

#[tokio::test]
async fn disconnect_waits_for_truthful_finalizing_commit_before_sftp_close() {
    let (_profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session_id = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, session_id);
    client.complete_upload.store(true, Ordering::SeqCst);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("finalizing.bin");
    std::fs::write(&source, b"12345678").unwrap();
    let plan = app
        .plan_upload(
            host.id,
            session_id,
            client.info.id,
            vec![
                nexus_sftp::open_local_source(
                    source.canonicalize().unwrap(),
                    "finalizing.bin".into(),
                )
                .unwrap(),
            ],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, session_id, client.info.id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.commit_started.notified(),
    )
    .await
    .expect("commit started");
    let disconnect_app = app.clone();
    let disconnecting = tokio::spawn(async move { disconnect_app.disconnect_host(host.id).await });
    tokio::task::yield_now().await;
    assert_eq!(
        app.list_transfers(Some(host.id))[0].state,
        TransferState::Finalizing
    );
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    assert!(
        !app.slot(host.id)
            .await
            .data
            .lock()
            .await
            .cancel
            .is_cancelled()
    );
    client.commit_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), disconnecting)
        .await
        .expect("disconnect finished")
        .unwrap()
        .unwrap();
    assert_eq!(
        app.list_transfers(Some(host.id))[0].state,
        TransferState::Completed
    );
    assert_eq!(client.cleanup_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    assert_eq!(job.id, app.list_transfers(Some(host.id))[0].id);
}

#[tokio::test]
async fn failed_owned_cleanup_blocks_disconnect_and_reports_failed_transfer() {
    let (_profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session_id = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, session_id);
    client.stall_owned_upload.store(true, Ordering::SeqCst);
    client.fail_cleanup.store(true, Ordering::SeqCst);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cleanup-failure.bin");
    std::fs::write(&source, b"12345678").unwrap();
    let plan = app
        .plan_upload(
            host.id,
            session_id,
            client.info.id,
            vec![
                nexus_sftp::open_local_source(
                    source.canonicalize().unwrap(),
                    "cleanup-failure.bin".into(),
                )
                .unwrap(),
            ],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, session_id, client.info.id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.upload_started.notified(),
    )
    .await
    .expect("upload started");
    let disconnect_app = app.clone();
    let disconnecting = tokio::spawn(async move { disconnect_app.disconnect_host(host.id).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .expect("cleanup attempted");
    client.cleanup_release.notify_one();
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), disconnecting)
        .await
        .expect("disconnect returned")
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Transfer);
    assert_eq!(client.cleanup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    assert!(app.sftp_sessions.lock().await.contains_key(&host.id));
    assert!(
        !app.slot(host.id)
            .await
            .data
            .lock()
            .await
            .cancel
            .is_cancelled()
    );
    let failed = wait_for_transfer(&app, job.id).await;
    assert_eq!(failed.state, TransferState::Failed);
    assert!(failed.error.unwrap().message.contains("staging cleanup"));
    #[cfg(windows)]
    std::fs::rename(&source, directory.path().join("source-released.bin")).unwrap();
}

async fn start_stalled_lifecycle_upload() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<Application>,
    HostId,
    Arc<LifecycleSftpClient>,
    TransferJobId,
    std::path::PathBuf,
) {
    let (profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let session_id = app
        .slot(host.id)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    let client = LifecycleSftpClient::new(host.id, session_id);
    client.stall_owned_upload.store(true, Ordering::SeqCst);
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let sources = tempfile::tempdir().unwrap();
    let source = sources.path().join("active.bin");
    std::fs::write(&source, b"12345678").unwrap();
    let plan = app
        .plan_upload(
            host.id,
            session_id,
            client.info.id,
            vec![
                nexus_sftp::open_local_source(source.canonicalize().unwrap(), "active.bin".into())
                    .unwrap(),
            ],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, session_id, client.info.id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.upload_started.notified(),
    )
    .await
    .expect("owned upload started");
    (profile, sources, app, host.id, client, job.id, source)
}

#[tokio::test]
async fn delete_waits_for_owned_cleanup_before_removing_host() {
    let (_profile, sources, app, host, client, job, source) =
        start_stalled_lifecycle_upload().await;
    let delete_app = app.clone();
    let deleting = tokio::spawn(async move { delete_app.delete_host(host).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .expect("cleanup started");
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    assert!(app.repository.get(host).is_ok());
    client.cleanup_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), deleting)
        .await
        .expect("delete finished")
        .unwrap()
        .unwrap();
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    assert!(app.repository.get(host).is_err());
    assert_eq!(
        wait_for_transfer(&app, job).await.state,
        TransferState::Cancelled
    );
    #[cfg(windows)]
    std::fs::rename(&source, sources.path().join("source-released.bin")).unwrap();
    #[cfg(not(windows))]
    let _ = (sources, source);
}

#[tokio::test]
async fn shutdown_waits_for_owned_cleanup_before_closing_sftp() {
    let (_profile, sources, app, host, client, job, source) =
        start_stalled_lifecycle_upload().await;
    let shutdown_app = app.clone();
    let shutting_down = tokio::spawn(async move { shutdown_app.shutdown().await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .expect("cleanup started");
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    assert!(!app.slot(host).await.data.lock().await.cancel.is_cancelled());
    client.cleanup_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), shutting_down)
        .await
        .expect("shutdown finished")
        .unwrap()
        .unwrap();
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    assert_eq!(
        wait_for_transfer(&app, job).await.state,
        TransferState::Cancelled
    );
    #[cfg(windows)]
    std::fs::rename(&source, sources.path().join("source-released.bin")).unwrap();
    #[cfg(not(windows))]
    let _ = (sources, source);
}

#[tokio::test]
async fn reconnect_quiesces_old_transfer_before_new_session_identity() {
    let (_profile, sources, app, host, client, job, source) =
        start_stalled_lifecycle_upload().await;
    let old_session = client.info.host_session_id;
    let reconnect_app = app.clone();
    let reconnecting = tokio::spawn(async move { reconnect_app.reconnect_host(host).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.cleanup_started.notified(),
    )
    .await
    .expect("cleanup started");
    assert_eq!(client.closes.load(Ordering::SeqCst), 0);
    client.cleanup_release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), reconnecting)
        .await
        .expect("reconnect finished")
        .unwrap()
        .unwrap();
    let new_session = app
        .slot(host)
        .await
        .data
        .lock()
        .await
        .connection_id
        .unwrap();
    assert_ne!(new_session, old_session);
    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
    assert_eq!(
        wait_for_transfer(&app, job).await.state,
        TransferState::Cancelled
    );
    assert!(
        app.cancel_transfer(host, old_session, client.info.id, job)
            .await
            .is_err()
    );
    #[cfg(windows)]
    std::fs::rename(&source, sources.path().join("source-released.bin")).unwrap();
    #[cfg(not(windows))]
    let _ = (sources, source);
}

async fn wait_for_transfer(app: &Application, id: TransferJobId) -> TransferJob {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(job) = app.list_transfers(None).into_iter().find(|job| {
                job.id == id
                    && matches!(
                        job.state,
                        TransferState::Failed
                            | TransferState::Cancelled
                            | TransferState::OutcomeUnknown
                    )
            }) {
                return job;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("transfer completion watchdog")
}

#[tokio::test]
async fn core_retry_boundary_releases_authority_and_requires_new_selection() {
    let (_profile, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.connect_host(host.id).await.unwrap();
    let slot = app.slot(host.id).await;
    let host_session_id = slot.data.lock().await.connection_id.unwrap();
    let client = LifecycleSftpClient::new(host.id, host_session_id);
    let sftp_session_id = client.info.id;
    app.sftp_sessions
        .lock()
        .await
        .insert(host.id, client.clone());
    let sources = tempfile::tempdir().unwrap();
    let path = sources.path().join("retry.bin");
    std::fs::write(&path, b"retry payload").unwrap();
    let path = path.canonicalize().unwrap();

    client.upload_failures.lock().unwrap().push_back(
        nexus_sftp::StagedUploadFailure::CreationOutcomeUnknown(AppError::new(
            ErrorCode::OutcomeUnknown,
            "Create reply lost.",
        )),
    );
    let plan = app
        .plan_upload(
            host.id,
            host_session_id,
            sftp_session_id,
            vec![nexus_sftp::open_local_source(path.clone(), "retry.bin".into()).unwrap()],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, host_session_id, sftp_session_id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    assert_eq!(
        wait_for_transfer(&app, job.id).await.state,
        TransferState::OutcomeUnknown
    );
    let before_retry = client.identity_calls.load(Ordering::SeqCst);
    assert_eq!(
        app.plan_retry_transfer(host.id, host_session_id, sftp_session_id, job.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Policy
    );
    assert_eq!(client.identity_calls.load(Ordering::SeqCst), before_retry);

    client
        .upload_failures
        .lock()
        .unwrap()
        .push_back(nexus_sftp::StagedUploadFailure::NotCreated(AppError::new(
            ErrorCode::Conflict,
            "Exclusive create refused.",
        )));
    let plan = app
        .plan_upload(
            host.id,
            host_session_id,
            sftp_session_id,
            vec![nexus_sftp::open_local_source(path.clone(), "retry.bin".into()).unwrap()],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, host_session_id, sftp_session_id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    let failed = wait_for_transfer(&app, job.id).await;
    assert_eq!(failed.state, TransferState::Failed);
    assert!(!failed.retryable);
    assert_eq!(
        app.plan_retry_transfer(host.id, host_session_id, sftp_session_id, job.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Policy
    );

    client.upload_failures.lock().unwrap().push_back(
        nexus_sftp::StagedUploadFailure::OwnedFailure(AppError::new(
            ErrorCode::Transfer,
            "Retryable stream failure.",
        )),
    );
    let plan = app
        .plan_upload(
            host.id,
            host_session_id,
            sftp_session_id,
            vec![nexus_sftp::open_local_source(path, "retry.bin".into()).unwrap()],
            "/tmp".into(),
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    let job = app
        .execute_file_plan(host.id, host_session_id, sftp_session_id, plan.id)
        .await
        .unwrap()[0]
        .clone();
    assert!(!wait_for_transfer(&app, job.id).await.retryable);
    assert_eq!(
        app.plan_retry_transfer(host.id, host_session_id, sftp_session_id, job.id)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Policy
    );
}
