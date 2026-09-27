use super::*;
use std::{future::Future, task::Poll};

struct Forbidden;
impl SecretStore for Forbidden {
    fn get(&self, _: HostId) -> Result<Credential, AppError> {
        panic!("trust read accessed secrets")
    }
    fn put(&self, _: HostId, _: &Credential) -> Result<(), AppError> {
        panic!("trust read wrote secrets")
    }
    fn delete(&self, _: HostId) -> Result<(), AppError> {
        panic!("trust read deleted secrets")
    }
}
#[async_trait]
impl ConnectionProvider for Forbidden {
    async fn connect(
        &self,
        _: &Host,
        _: Credential,
        _: CancellationToken,
    ) -> Result<Arc<dyn ConnectedTransport>, AppError> {
        panic!("trust read connected")
    }
}
#[async_trait]
impl RemoteSession for Forbidden {
    async fn execute(&self, _: ReadOnlyCommand, _: CancellationToken) -> Result<String, AppError> {
        panic!("trust read executed remote command")
    }
    async fn disconnect(&self) -> Result<(), AppError> {
        panic!("trust read disconnected")
    }
    fn is_closed(&self) -> bool {
        panic!("trust read reconciled session")
    }
}
#[async_trait]
impl TerminalConnector for Forbidden {
    async fn open_terminal(
        &self,
        _: TerminalSize,
        _: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError> {
        panic!("trust read opened terminal")
    }
}
#[async_trait]
impl nexus_sftp::SftpConnector for Forbidden {
    async fn open_sftp(
        &self,
        _: HostId,
        _: HostSessionId,
    ) -> Result<Arc<dyn nexus_sftp::SftpClient>, AppError> {
        panic!("trust read opened SFTP")
    }
}

fn pin(hostname: &str, port: u16) -> HostKeyChallenge {
    HostKeyChallenge {
        hostname: hostname.into(),
        port,
        algorithm: "ssh-ed25519".into(),
        fingerprint: format!("SHA256:{}", "A".repeat(43)),
        previous_fingerprint: None,
    }
}

#[tokio::test]
async fn ssh_trust_is_offline_metadata_without_secrets_transport_audit_or_pin_mutation() {
    let (dir, mut app, _) = setup(false);
    let writable = Arc::get_mut(&mut app).expect("unique app");
    writable.secrets = Arc::new(Forbidden);
    writable.provider = Arc::new(Forbidden);
    for method in [
        AuthenticationMethod::Password,
        AuthenticationMethod::PrivateKey,
    ] {
        let mut host = input().into_host().unwrap();
        host.connection.authentication = method;
        // Configured metadata must work even without a credential record.
        app.repository
            .save(&StoredHost {
                host: host.clone(),
                credential_id: HostId::new(),
            })
            .unwrap();
        let before_audit = std::fs::read(dir.path().join("audit.jsonl")).unwrap();
        let empty = app.get_host_ssh_trust(host.id).await.unwrap();
        assert_eq!(empty.host_id, host.id);
        assert_eq!(empty.hostname, host.connection.hostname);
        assert_eq!(empty.port, 22);
        assert_eq!(empty.authentication, method);
        assert_eq!(empty.endpoint_pin, None);
        assert!(app.sessions.lock().await.is_empty());
        assert_eq!(app.known_hosts.fingerprint("localhost", 22).unwrap(), None);

        app.known_hosts.trust(&pin("localhost", 22)).unwrap();
        let stored = app.known_hosts.fingerprint("localhost", 22).unwrap();
        // Even with a connected transport, this path must not inspect or reconcile it.
        let slot = app.slot(host.id).await;
        {
            let mut data = slot.data.lock().await;
            data.view.state = ConnectionState::Connected;
            data.transport = Some(Arc::new(Forbidden));
        }
        let trust = app.get_host_ssh_trust(host.id).await.unwrap();
        assert_eq!(trust.endpoint_pin, stored);
        assert_eq!(
            app.known_hosts.fingerprint("localhost", 22).unwrap(),
            stored
        );
        assert_eq!(
            std::fs::read(dir.path().join("audit.jsonl")).unwrap(),
            before_audit
        );
        let json = serde_json::to_value(trust).unwrap();
        let keys = json.as_object().unwrap();
        assert_eq!(keys.len(), 5);
        for key in [
            "hostId",
            "hostname",
            "port",
            "authentication",
            "endpointPin",
        ] {
            assert!(keys.contains_key(key));
        }
        app.sessions.lock().await.clear();
        // Reset only this test's pin fixture so each method starts with real absence.
        let db = rusqlite::Connection::open(dir.path().join("pins.db")).unwrap();
        db.execute("DELETE FROM ssh_host_keys", []).unwrap();
    }
    assert_eq!(
        app.get_host_ssh_trust(HostId::new())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn ssh_trust_pin_is_shared_by_endpoint_not_host_id_and_survives_delete_recreate() {
    let (_dir, app, _) = setup(false);
    let first = app.save_host(input(), Some(credential())).await.unwrap();
    app.known_hosts.trust(&pin("LOCALHOST.", 22)).unwrap();
    let expected = app.get_host_ssh_trust(first.id).await.unwrap().endpoint_pin;
    let same = app.save_host(input(), Some(credential())).await.unwrap();
    assert_ne!(same.id, first.id);
    assert_eq!(
        app.get_host_ssh_trust(same.id).await.unwrap().endpoint_pin,
        expected
    );
    for (hostname, port) in [("localhost", 2222), ("127.0.0.1", 22)] {
        let mut other = input();
        other.connection.hostname = hostname.into();
        other.connection.port = port;
        let other = app.save_host(other, Some(credential())).await.unwrap();
        assert_eq!(
            app.get_host_ssh_trust(other.id).await.unwrap().endpoint_pin,
            None
        );
    }
    app.delete_host(first.id).await.unwrap();
    assert_eq!(
        app.get_host_ssh_trust(first.id).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    let replacement = app.save_host(input(), Some(credential())).await.unwrap();
    assert_eq!(
        app.get_host_ssh_trust(replacement.id)
            .await
            .unwrap()
            .endpoint_pin,
        expected
    );
    let mut edit = input();
    edit.id = Some(replacement.id);
    edit.connection.hostname = "new.example".into();
    app.save_host(edit, None).await.unwrap();
    let updated = app.get_host_ssh_trust(replacement.id).await.unwrap();
    assert_eq!(updated.hostname, "new.example");
    assert_eq!(updated.endpoint_pin, None);
    assert_eq!(
        app.known_hosts.fingerprint("localhost", 22).unwrap(),
        expected
    );
}

#[tokio::test]
async fn ssh_trust_rejects_corrupt_host_and_pin_storage_without_echoing_values() {
    let (dir, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    let db = rusqlite::Connection::open(dir.path().join("hosts.db")).unwrap();
    db.execute("UPDATE hosts SET hostname=?1", ["bad\n\u{202e}host"])
        .unwrap();
    let error = app.get_host_ssh_trust(host.id).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Persistence);
    assert_eq!(
        error.message,
        "The stored SSH endpoint configuration is invalid."
    );
    assert!(error.host_key.is_none());
    db.execute("UPDATE hosts SET hostname='localhost'", [])
        .unwrap();
    app.known_hosts.trust(&pin("localhost", 22)).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("pins.db")).unwrap();
    for (algorithm, fingerprint) in [
        ("x".repeat(129), pin("localhost", 22).fingerprint),
        ("ssh\ned25519".into(), pin("localhost", 22).fingerprint),
        (
            "ssh\u{202e}ed25519".into(),
            pin("localhost", 22).fingerprint,
        ),
        ("ssh-ed25519".into(), format!("SHA512:{}", "A".repeat(43))),
        ("ssh-ed25519".into(), "SHA256:short".into()),
        ("ssh-ed25519".into(), format!("SHA256:{}", "!".repeat(43))),
    ] {
        db.execute(
            "UPDATE ssh_host_keys SET algorithm=?1, fingerprint=?2",
            rusqlite::params![algorithm, fingerprint],
        )
        .unwrap();
        let error = app.get_host_ssh_trust(host.id).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Persistence);
        assert_eq!(error.message, "The trusted SSH host key store is invalid.");
        assert!(error.host_key.is_none());
    }
}

async fn assert_pending<F: Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn ssh_trust_reads_serialize_with_edit_trust_and_delete() {
    let (_dir, app, _) = setup(false);
    let host = app.save_host(input(), Some(credential())).await.unwrap();
    app.known_hosts.trust(&pin("localhost", 22)).unwrap();
    let mut edit = input();
    edit.id = Some(host.id);
    edit.connection.hostname = "new.example".into();
    let gate = app.mutation.lock().await;
    let mut edit = Box::pin(app.save_host(edit, None));
    assert_pending(edit.as_mut()).await;
    let mut read = Box::pin(app.get_host_ssh_trust(host.id));
    assert_pending(read.as_mut()).await;
    drop(gate);
    let (edited, read) = tokio::join!(edit, read);
    let edited = edited.unwrap();
    let read = read.unwrap();
    assert_eq!(read.hostname, edited.connection.hostname);
    assert_eq!(read.endpoint_pin, None);

    let challenge = pin("new.example", 22);
    {
        let slot = app.slot(host.id).await;
        let mut data = slot.data.lock().await;
        data.view.state = ConnectionState::AwaitingTrust;
        data.view.error = Some(AppError {
            code: ErrorCode::UnknownHostKey,
            message: "Test challenge".into(),
            host_key: Some(Box::new(challenge.clone())),
        });
    }
    let gate = app.mutation.lock().await;
    let mut trust = Box::pin(app.trust_host_key(host.id, challenge.clone()));
    assert_pending(trust.as_mut()).await;
    let mut read = Box::pin(app.get_host_ssh_trust(host.id));
    assert_pending(read.as_mut()).await;
    drop(gate);
    let (trusted, read) = tokio::join!(trust, read);
    trusted.unwrap();
    let read = read.unwrap();
    assert_eq!(read.hostname, challenge.hostname);
    assert_eq!(read.endpoint_pin.unwrap().sha256, challenge.fingerprint);

    let gate = app.mutation.lock().await;
    let mut delete = Box::pin(app.delete_host(host.id));
    assert_pending(delete.as_mut()).await;
    let mut read = Box::pin(app.get_host_ssh_trust(host.id));
    assert_pending(read.as_mut()).await;
    drop(gate);
    let (deleted, read) = tokio::join!(delete, read);
    deleted.unwrap();
    assert_eq!(read.unwrap_err().code, ErrorCode::NotFound);
}
