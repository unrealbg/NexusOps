//! Opt-in interoperability tests against the disposable OpenSSH fixture in
//! `tools/openssh-fixture`. These tests use NexusOps' production SSH provider.

use nexus_model::{AuthenticationMethod, ErrorCode, Host, HostConnectionConfig, HostId};
use nexus_operations::{ReadOnlyCommand, RemoteSession};
use nexus_secrets::Credential;
use nexus_ssh::{KnownHosts, SshProvider};
use std::{
    env, fs,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

struct Fixture {
    host: String,
    port: u16,
    user: String,
    password: Zeroizing<String>,
    private_key: Zeroizing<String>,
    protected_key: Zeroizing<String>,
    passphrase: Zeroizing<String>,
    pin_db: String,
    log: String,
    fingerprint_a: String,
}

impl Fixture {
    fn from_env() -> Self {
        let value = |name: &str| {
            env::var(name).unwrap_or_else(|_| {
                panic!("missing {name}; run tools/openssh-fixture/Run-OpenSshInterop.ps1")
            })
        };
        Self {
            host: value("NEXUS_OPENSSH_HOST"),
            port: value("NEXUS_OPENSSH_PORT").parse().expect("port"),
            user: value("NEXUS_OPENSSH_USER"),
            password: Zeroizing::new(value("NEXUS_OPENSSH_PASSWORD")),
            private_key: Zeroizing::new(
                fs::read_to_string(value("NEXUS_OPENSSH_PRIVATE_KEY")).expect("private key"),
            ),
            protected_key: Zeroizing::new(
                fs::read_to_string(value("NEXUS_OPENSSH_PROTECTED_KEY")).expect("protected key"),
            ),
            passphrase: Zeroizing::new(value("NEXUS_OPENSSH_KEY_PASSPHRASE")),
            pin_db: value("NEXUS_OPENSSH_PIN_DB"),
            log: value("NEXUS_OPENSSH_LOG"),
            fingerprint_a: value("NEXUS_OPENSSH_FINGERPRINT_A"),
        }
    }

    fn host(&self, authentication: AuthenticationMethod) -> Host {
        Host {
            id: HostId::new(),
            display_name: "Disposable OpenSSH fixture".into(),
            connection: HostConnectionConfig {
                hostname: self.host.clone(),
                port: self.port,
                username: self.user.clone(),
                authentication,
            },
        }
    }

    fn password(&self, value: &str) -> Credential {
        Credential {
            password: Some(Zeroizing::new(value.into())),
            private_key: None,
            passphrase: None,
        }
    }

    fn key(&self, protected: bool, passphrase: Option<&str>) -> Credential {
        Credential {
            password: None,
            private_key: Some(Zeroizing::new(if protected {
                self.protected_key.to_string()
            } else {
                self.private_key.to_string()
            })),
            passphrase: passphrase.map(|value| Zeroizing::new(value.into())),
        }
    }
}

async fn hostname(session: &dyn RemoteSession) -> String {
    session
        .execute(ReadOnlyCommand::Hostname, CancellationToken::new())
        .await
        .expect("hostname")
}

#[tokio::test]
#[ignore = "requires tools/openssh-fixture disposable real OpenSSH server"]
async fn openssh_phase_a() {
    let fixture = Fixture::from_env();
    assert_ne!(fixture.port, 22, "fixture must use a non-default port");
    let _ = fs::remove_file(&fixture.pin_db);
    let store = Arc::new(KnownHosts::open(&fixture.pin_db).expect("known hosts"));
    let provider = SshProvider::new(store.clone());
    let host = fixture.host(AuthenticationMethod::Password);
    let before_log = fs::read_to_string(&fixture.log).unwrap_or_default();

    let unknown = provider
        .connect(
            &host,
            fixture.password(&fixture.password),
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("unknown key");
    assert_eq!(unknown.code, ErrorCode::UnknownHostKey);
    let challenge = unknown.host_key.expect("host-key challenge");
    assert_eq!(challenge.fingerprint, fixture.fingerprint_a);
    assert!(
        store
            .fingerprint(&fixture.host, fixture.port)
            .expect("lookup")
            .is_none(),
        "rejection must not persist trust"
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    let new_log = fs::read_to_string(&fixture.log).unwrap_or_default();
    let suffix = new_log.strip_prefix(&before_log).unwrap_or(&new_log);
    assert!(
        !suffix.contains("Accepted password")
            && !suffix.contains("Accepted publickey")
            && !suffix.contains("Starting session"),
        "authentication or discovery occurred before trust"
    );

    store.trust(&challenge).expect("explicit trust");
    let bad_password = provider
        .connect(
            &host,
            fixture.password("incorrect-password"),
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("bad password");
    assert_eq!(bad_password.code, ErrorCode::Authentication);

    let password_session = provider
        .connect(
            &host,
            fixture.password(&fixture.password),
            CancellationToken::new(),
        )
        .await
        .expect("password authentication");
    let snapshot = nexus_discovery::discover(password_session.as_ref(), CancellationToken::new())
        .await
        .expect("real discovery");
    assert_eq!(snapshot.os.as_deref(), Some("Alpine Linux"));
    assert!(snapshot.kernel.is_some() && snapshot.architecture.is_some());
    assert!(!snapshot.observed_at.is_empty());

    let key_host = fixture.host(AuthenticationMethod::PrivateKey);
    let key_session = provider
        .connect(
            &key_host,
            fixture.key(false, None),
            CancellationToken::new(),
        )
        .await
        .expect("Ed25519 authentication");
    assert!(!hostname(key_session.as_ref()).await.trim().is_empty());
    let protected_session = provider
        .connect(
            &key_host,
            fixture.key(true, Some(&fixture.passphrase)),
            CancellationToken::new(),
        )
        .await
        .expect("protected key authentication");
    let wrong_passphrase = provider
        .connect(
            &key_host,
            fixture.key(true, Some("incorrect-passphrase")),
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("wrong passphrase");
    assert_eq!(wrong_passphrase.code, ErrorCode::Authentication);

    password_session
        .disconnect()
        .await
        .expect("first disconnect");
    assert!(password_session.is_closed());
    assert!(!key_session.is_closed());
    assert!(
        !hostname(key_session.as_ref()).await.trim().is_empty(),
        "second session remains usable"
    );
    key_session.disconnect().await.expect("second disconnect");
    protected_session
        .disconnect()
        .await
        .expect("protected disconnect");

    drop(provider);
    drop(store);
    let reopened = Arc::new(KnownHosts::open(&fixture.pin_db).expect("reopen known hosts"));
    assert_eq!(
        reopened
            .fingerprint(&fixture.host, fixture.port)
            .expect("persisted lookup")
            .expect("persisted pin")
            .sha256,
        fixture.fingerprint_a
    );
    let restarted_provider = SshProvider::new(reopened);
    let first = restarted_provider
        .connect(
            &host,
            fixture.password(&fixture.password),
            CancellationToken::new(),
        )
        .await
        .expect("trusted reconnect after provider restart");
    first.disconnect().await.expect("disconnect");
    let second = restarted_provider
        .connect(
            &host,
            fixture.password(&fixture.password),
            CancellationToken::new(),
        )
        .await
        .expect("reconnect");
    second.disconnect().await.expect("disconnect");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stall listener");
    let stall_port = listener.local_addr().expect("address").port();
    let stall = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.expect("accept");
        tokio::time::sleep(Duration::from_secs(20)).await;
    });
    let mut unavailable = host.clone();
    unavailable.connection.port = stall_port;
    let started = Instant::now();
    let timeout_error = restarted_provider
        .connect(
            &unavailable,
            fixture.password(&fixture.password),
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("bounded timeout");
    assert_eq!(timeout_error.code, ErrorCode::Timeout);
    assert!(started.elapsed() < Duration::from_secs(17));
    stall.abort();
}

#[tokio::test]
#[ignore = "requires phase A pin and fixture restarted with host key B"]
async fn openssh_phase_b_changed_key() {
    let fixture = Fixture::from_env();
    let store = Arc::new(KnownHosts::open(&fixture.pin_db).expect("known hosts"));
    let original = store
        .fingerprint(&fixture.host, fixture.port)
        .expect("lookup")
        .expect("phase A pin");
    assert_eq!(original.sha256, fixture.fingerprint_a);
    let provider = SshProvider::new(store.clone());
    let error = provider
        .connect(
            &fixture.host(AuthenticationMethod::Password),
            fixture.password(&fixture.password),
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("changed key blocked");
    assert_eq!(error.code, ErrorCode::ChangedHostKey);
    let challenge = error.host_key.expect("changed challenge");
    assert_eq!(
        challenge.previous_fingerprint.as_deref(),
        Some(fixture.fingerprint_a.as_str())
    );
    assert_ne!(challenge.fingerprint, fixture.fingerprint_a);
    assert_eq!(
        store
            .trust(&challenge)
            .expect_err("changed key cannot be overwritten")
            .code,
        ErrorCode::ChangedHostKey
    );
    assert_eq!(
        store
            .fingerprint(&fixture.host, fixture.port)
            .expect("lookup")
            .expect("pin retained")
            .sha256,
        fixture.fingerprint_a
    );
}
