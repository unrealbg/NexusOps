//! Real SSH protocol tests bound exclusively to ephemeral loopback ports.
//! The fixture returns deterministic Linux text; it never invokes a shell.

#[path = "support/probe_responses.rs"]
mod probe_responses;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nexus_model::{
    AuthenticationMethod, ErrorCode, Host, HostConnectionConfig, HostFingerprint, HostId,
    HostKeyChallenge,
};
use nexus_operations::{ReadOnlyCommand, RemoteSession};
use nexus_secrets::Credential;
use nexus_ssh::{KnownHosts, SshProvider};
use russh::{
    Channel, ChannelId,
    keys::{Algorithm, HashAlg, PrivateKey, PublicKey, ssh_key::LineEnding},
    server::{self, Auth, Server as _},
};
use tokio::{
    net::TcpListener,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[derive(Clone)]
struct FixtureHandler {
    auth_attempts: Arc<AtomicUsize>,
    command_count: Arc<AtomicUsize>,
    channel_closes: Arc<AtomicUsize>,
    mode: Arc<AtomicU8>,
    approved_key: PublicKey,
}

impl server::Server for FixtureHandler {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for FixtureHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "nexus" && password == "loopback-only-password" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        self.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "nexus" && key == &self.approved_key {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.channel_closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.command_count.fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel)?;
        match self.mode.load(Ordering::SeqCst) {
            1 => {
                // Output is sent incrementally by russh with SSH window backpressure.
                session.data(channel, vec![b'x'; 128 * 1024])?;
            }
            2 => return Ok(()), // Deliberately stalls until the client closes the channel.
            3 => {
                session.exit_status_request(channel, 1)?;
                session.close(channel)?;
                return Ok(());
            }
            _ => {
                let Some(text) = probe_responses::response(command) else {
                    session.channel_failure(channel)?;
                    session.close(channel)?;
                    return Ok(());
                };
                session.data(channel, text.as_bytes())?;
            }
        }
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

struct Fixture {
    host: Host,
    fingerprint: HostFingerprint,
    user_key: PrivateKey,
    handler: FixtureHandler,
    cancellation: CancellationToken,
}

impl Fixture {
    async fn start() -> Self {
        let server_key =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("server key");
        let user_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("user key");
        let fingerprint = HostFingerprint {
            algorithm: server_key.algorithm().to_string(),
            sha256: server_key
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener");
        let port = listener.local_addr().expect("address").port();
        let handler = FixtureHandler {
            auth_attempts: Arc::new(AtomicUsize::new(0)),
            command_count: Arc::new(AtomicUsize::new(0)),
            channel_closes: Arc::new(AtomicUsize::new(0)),
            mode: Arc::new(AtomicU8::new(0)),
            approved_key: user_key.public_key().clone(),
        };
        let mut server = handler.clone();
        let cancellation = CancellationToken::new();
        let stop = cancellation.clone();
        tokio::spawn(async move {
            let config = Arc::new(server::Config {
                keys: vec![server_key],
                auth_rejection_time: Duration::from_millis(5),
                auth_rejection_time_initial: Some(Duration::from_millis(5)),
                ..Default::default()
            });
            let running = server.run_on_socket(config, &listener);
            let shutdown = running.handle();
            tokio::select! {
                result = running => result.expect("fixture server"),
                _ = stop.cancelled() => shutdown.shutdown("test finished".into()),
            }
        });
        Self {
            host: Host {
                id: HostId::new(),
                display_name: "Loopback SSH fixture".into(),
                connection: HostConnectionConfig {
                    hostname: "127.0.0.1".into(),
                    port,
                    username: "nexus".into(),
                    authentication: AuthenticationMethod::Password,
                },
            },
            fingerprint,
            user_key,
            handler,
            cancellation,
        }
    }

    fn trusted_provider(&self) -> SshProvider {
        let store = Arc::new(KnownHosts::open(":memory:").expect("pins"));
        store
            .trust(&HostKeyChallenge {
                hostname: self.host.connection.hostname.clone(),
                port: self.host.connection.port,
                algorithm: self.fingerprint.algorithm.clone(),
                fingerprint: self.fingerprint.sha256.clone(),
                previous_fingerprint: None,
            })
            .expect("trust fixture");
        SshProvider::new(store)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn password() -> Credential {
    Credential {
        password: Some(Zeroizing::new("loopback-only-password".into())),
        private_key: None,
        passphrase: None,
    }
}

#[tokio::test]
async fn unknown_key_blocks_before_auth_then_explicit_trust_allows_discovery() {
    let fixture = Fixture::start().await;
    let store = Arc::new(KnownHosts::open(":memory:").expect("pins"));
    let provider = SshProvider::new(store.clone());
    let error = provider
        .connect(&fixture.host, password(), CancellationToken::new())
        .await
        .err()
        .expect("unknown key");
    assert_eq!(error.code, ErrorCode::UnknownHostKey);
    assert_eq!(fixture.handler.auth_attempts.load(Ordering::SeqCst), 0);
    let challenge = error.host_key.expect("challenge");
    assert_eq!(challenge.fingerprint, fixture.fingerprint.sha256);
    store.trust(&challenge).expect("explicit trust");
    let session = provider
        .connect(&fixture.host, password(), CancellationToken::new())
        .await
        .expect("connect");
    let snapshot = nexus_discovery::discover(session.as_ref(), CancellationToken::new())
        .await
        .expect("discovery");
    assert_eq!(snapshot.hostname.as_deref(), Some("nexus-fixture"));
    assert_eq!(snapshot.kernel.as_deref(), Some("6.12.0-fixture"));
    assert_eq!(snapshot.architecture.as_deref(), Some("x86_64"));
    assert_eq!(snapshot.load_one, Some(0.25));
    assert_eq!(snapshot.memory_total_bytes, Some(8192000 * 1024));
    assert_eq!(snapshot.root_used_bytes, Some(250000 * 1024));
    assert!(
        snapshot.warnings.is_empty(),
        "discovery warnings: {:?}",
        snapshot.warnings
    );
    assert_eq!(fixture.handler.command_count.load(Ordering::SeqCst), 8);
    session.disconnect().await.expect("disconnect");
    assert!(session.is_closed());
}

#[tokio::test]
async fn changed_actual_server_key_is_rejected_before_authentication() {
    let fixture = Fixture::start().await;
    let store = Arc::new(KnownHosts::open(":memory:").expect("pins"));
    store
        .trust(&HostKeyChallenge {
            hostname: "127.0.0.1".into(),
            port: fixture.host.connection.port,
            algorithm: "ssh-ed25519".into(),
            fingerprint: format!("SHA256:{}", "A".repeat(43)),
            previous_fingerprint: None,
        })
        .expect("old pin");
    let error = SshProvider::new(store)
        .connect(&fixture.host, password(), CancellationToken::new())
        .await
        .err()
        .expect("changed key");
    assert_eq!(error.code, ErrorCode::ChangedHostKey);
    assert_eq!(fixture.handler.auth_attempts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn private_key_authentication_and_rejected_password_are_distinct() {
    let fixture = Fixture::start().await;
    let provider = fixture.trusted_provider();
    let rejected = Credential {
        password: Some(Zeroizing::new("incorrect".into())),
        private_key: None,
        passphrase: None,
    };
    let error = provider
        .connect(&fixture.host, rejected, CancellationToken::new())
        .await
        .err()
        .expect("rejected password");
    assert_eq!(error.code, ErrorCode::Authentication);
    let mut host = fixture.host.clone();
    host.connection.authentication = AuthenticationMethod::PrivateKey;
    let private_key = fixture
        .user_key
        .to_openssh(LineEnding::LF)
        .expect("encode key");
    let credential = Credential {
        password: None,
        private_key: Some(Zeroizing::new(private_key.as_str().into())),
        passphrase: None,
    };
    let session = provider
        .connect(&host, credential, CancellationToken::new())
        .await
        .expect("private key connect");
    assert_eq!(
        session
            .execute(ReadOnlyCommand::Hostname, CancellationToken::new())
            .await
            .expect("command"),
        "nexus-fixture\n"
    );
    session.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn encrypted_private_key_requires_the_correct_passphrase() {
    let fixture = Fixture::start().await;
    let provider = fixture.trusted_provider();
    let mut host = fixture.host.clone();
    host.connection.authentication = AuthenticationMethod::PrivateKey;
    let encrypted_key = fixture
        .user_key
        .encrypt(&mut rand::rng(), "fixture-passphrase")
        .expect("encrypt fixture key")
        .to_openssh(LineEnding::LF)
        .expect("encode key");
    let credential = |passphrase: &str| Credential {
        password: None,
        private_key: Some(Zeroizing::new(encrypted_key.as_str().into())),
        passphrase: Some(Zeroizing::new(passphrase.into())),
    };
    let error = provider
        .connect(&host, credential("incorrect"), CancellationToken::new())
        .await
        .err()
        .expect("wrong passphrase");
    assert_eq!(error.code, ErrorCode::Authentication);
    assert_eq!(fixture.handler.auth_attempts.load(Ordering::SeqCst), 0);
    let session = provider
        .connect(
            &host,
            credential("fixture-passphrase"),
            CancellationToken::new(),
        )
        .await
        .expect("encrypted key authentication");
    assert_eq!(
        session
            .execute(ReadOnlyCommand::Hostname, CancellationToken::new())
            .await
            .expect("command"),
        "nexus-fixture\n"
    );
    session.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn independent_hosts_do_not_share_session_lifetime() {
    let first = Fixture::start().await;
    let second = Fixture::start().await;
    let first_session = first
        .trusted_provider()
        .connect(&first.host, password(), CancellationToken::new())
        .await
        .expect("first host");
    let second_session = second
        .trusted_provider()
        .connect(&second.host, password(), CancellationToken::new())
        .await
        .expect("second host");
    first_session.disconnect().await.expect("first disconnect");
    assert!(first_session.is_closed());
    assert!(!second_session.is_closed());
    assert_eq!(
        second_session
            .execute(ReadOnlyCommand::Hostname, CancellationToken::new())
            .await
            .expect("second still usable"),
        "nexus-fixture\n"
    );
    second_session
        .disconnect()
        .await
        .expect("second disconnect");
}

#[tokio::test]
async fn output_limit_and_optional_probe_failure_preserve_session() {
    let fixture = Fixture::start().await;
    let session = fixture
        .trusted_provider()
        .connect(&fixture.host, password(), CancellationToken::new())
        .await
        .expect("connect");
    fixture.handler.mode.store(1, Ordering::SeqCst);
    assert_eq!(
        session
            .execute(ReadOnlyCommand::Hostname, CancellationToken::new())
            .await
            .expect_err("bounded output")
            .code,
        ErrorCode::Discovery
    );
    fixture.handler.mode.store(3, Ordering::SeqCst);
    let snapshot = nexus_discovery::discover(session.as_ref(), CancellationToken::new())
        .await
        .expect("optional failures");
    assert_eq!(snapshot.warnings.len(), 8);
    assert!(!session.is_closed());
    fixture.handler.mode.store(0, Ordering::SeqCst);
    assert_eq!(
        session
            .execute(ReadOnlyCommand::Hostname, CancellationToken::new())
            .await
            .expect("healthy again"),
        "nexus-fixture\n"
    );
    session.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn cancelling_operation_closes_channel_and_lifetime_closes_session() {
    let fixture = Fixture::start().await;
    let lifetime = CancellationToken::new();
    let session = fixture
        .trusted_provider()
        .connect(&fixture.host, password(), lifetime.clone())
        .await
        .expect("connect");
    fixture.handler.mode.store(2, Ordering::SeqCst);
    let operation = CancellationToken::new();
    let execution = tokio::spawn({
        let session = session.clone();
        let token = operation.clone();
        async move { session.execute(ReadOnlyCommand::Hostname, token).await }
    });
    timeout(Duration::from_secs(2), async {
        while fixture.handler.command_count.load(Ordering::SeqCst) == 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("command began");
    operation.cancel();
    assert_eq!(
        timeout(Duration::from_secs(2), execution)
            .await
            .expect("bounded cancellation")
            .expect("task")
            .expect_err("cancelled")
            .code,
        ErrorCode::Cancelled
    );
    timeout(Duration::from_secs(2), async {
        while fixture.handler.channel_closes.load(Ordering::SeqCst) == 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("channel closed after cancellation");
    assert!(!session.is_closed());
    lifetime.cancel();
    assert!(session.is_closed());
}

#[tokio::test]
async fn pre_cancelled_connect_never_authenticates() {
    let fixture = Fixture::start().await;
    let token = CancellationToken::new();
    token.cancel();
    let error = fixture
        .trusted_provider()
        .connect(&fixture.host, password(), token)
        .await
        .err()
        .expect("cancelled");
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(fixture.handler.auth_attempts.load(Ordering::SeqCst), 0);
}
