//! Opt-in fixed-command monitoring validation against the owned OpenSSH fixture.

use nexus_model::{
    AuthenticationMethod, ErrorCode, Host, HostConnectionConfig, HostId, HostSessionId,
};
use nexus_operations::RemoteSession;
use nexus_secrets::Credential;
use nexus_ssh::{KnownHosts, SshProvider};
use std::{env, fs, sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[tokio::test]
#[ignore = "requires tools/openssh-fixture disposable real OpenSSH server"]
async fn openssh_monitoring_interoperability() {
    let value = |name: &str| env::var(name).unwrap_or_else(|_| panic!("missing {name}"));
    let host = Host {
        id: HostId::new(),
        display_name: "Disposable OpenSSH monitoring fixture".into(),
        connection: HostConnectionConfig {
            hostname: value("NEXUS_OPENSSH_HOST"),
            port: value("NEXUS_OPENSSH_PORT").parse().expect("port"),
            username: value("NEXUS_OPENSSH_USER"),
            authentication: AuthenticationMethod::Password,
        },
    };
    assert_ne!(
        host.connection.port, 22,
        "fixture must use a non-default port"
    );
    let password = Zeroizing::new(value("NEXUS_OPENSSH_PASSWORD"));
    let pins = Arc::new(KnownHosts::open(value("NEXUS_OPENSSH_PIN_DB")).expect("known hosts"));
    assert_eq!(
        pins.fingerprint(&host.connection.hostname, host.connection.port)
            .expect("pin lookup")
            .expect("phase A trusted fixture")
            .sha256,
        value("NEXUS_OPENSSH_FINGERPRINT_A")
    );
    let provider = SshProvider::new(pins);
    let session = provider
        .connect(
            &host,
            Credential {
                password: Some(password),
                private_key: None,
                passphrase: None,
            },
            CancellationToken::new(),
        )
        .await
        .expect("fixture connection");
    let host_session_id = HostSessionId::new();
    let first_at = Instant::now();
    let first = nexus_discovery::observe_monitor(session.as_ref(), CancellationToken::new())
        .await
        .expect("first observation");
    let (first, baseline) =
        nexus_discovery::derive_sample(host.id, host_session_id, first, None, None);
    assert!(first.cpu_usage_percent.is_none());
    assert!(first.network_rx_bytes_per_second.is_none());
    assert!(first.memory_total_bytes.is_some() && first.swap_total_bytes.is_some());
    assert!(first.root_total_bytes.is_some());

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let second = nexus_discovery::observe_monitor(session.as_ref(), CancellationToken::new())
        .await
        .expect("second observation");
    let (second, _) = nexus_discovery::derive_sample(
        host.id,
        host_session_id,
        second,
        Some(&baseline),
        Some(first_at.elapsed()),
    );
    let cpu = second.cpu_usage_percent.expect("CPU delta");
    assert!((0.0..=100.0).contains(&cpu));
    assert!(
        second
            .network_rx_bytes_per_second
            .is_some_and(|value| value >= 0.0)
    );
    assert!(
        second
            .network_tx_bytes_per_second
            .is_some_and(|value| value >= 0.0)
    );
    assert!(second.warnings.is_empty(), "fixture monitoring warnings");

    session.disconnect().await.expect("disconnect");
    let error = nexus_discovery::observe_monitor(session.as_ref(), CancellationToken::new())
        .await
        .expect_err("closed session cannot be sampled");
    assert_eq!(error.code, ErrorCode::Connection);
    drop(provider);
    let _ = fs::metadata(value("NEXUS_OPENSSH_LOG")).expect("owned fixture log");
}
