//! Opt-in PTY interoperability against the marker-owned OpenSSH fixture.
//! Terminal contents remain in test memory and are never printed or logged.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use nexus_model::{
    AuthenticationMethod, Host, HostConnectionConfig, HostId, HostSessionId, TerminalSession,
    TerminalSize, TerminalState,
};
use nexus_operations::RemoteSession;
use nexus_secrets::Credential;
use nexus_ssh::{KnownHosts, SshProvider};
use nexus_terminal::{OUTPUT_BATCH_BYTES, OUTPUT_CHUNK_BYTES, TerminalManager};
use std::{env, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

struct Fixture {
    host: Host,
    password: Zeroizing<String>,
    pin_db: String,
}

impl Fixture {
    fn from_env() -> Self {
        let value = |name: &str| {
            env::var(name).unwrap_or_else(|_| {
                panic!("missing {name}; run tools/openssh-fixture/Run-OpenSshInterop.ps1")
            })
        };
        Self {
            host: Host {
                id: HostId::new(),
                display_name: "Disposable OpenSSH terminal fixture".into(),
                connection: HostConnectionConfig {
                    hostname: value("NEXUS_OPENSSH_HOST"),
                    port: value("NEXUS_OPENSSH_PORT").parse().expect("port"),
                    username: value("NEXUS_OPENSSH_USER"),
                    authentication: AuthenticationMethod::Password,
                },
            },
            password: Zeroizing::new(value("NEXUS_OPENSSH_PASSWORD")),
            pin_db: value("NEXUS_OPENSSH_PIN_DB"),
        }
    }

    fn credential(&self) -> Credential {
        Credential {
            password: Some(self.password.clone()),
            private_key: None,
            passphrase: None,
        }
    }
}

fn size(columns: u32, rows: u32) -> TerminalSize {
    TerminalSize {
        columns,
        rows,
        pixel_width: columns * 9,
        pixel_height: rows * 18,
    }
}

async fn send(manager: &TerminalManager, terminal: &TerminalSession, bytes: &[u8]) {
    manager
        .write(
            (terminal.host_id, terminal.host_session_id, terminal.id),
            &STANDARD.encode(bytes),
        )
        .await
        .expect("terminal input");
}

async fn collect_until(
    manager: &TerminalManager,
    terminal: &TerminalSession,
    marker: &[u8],
    minimum_bytes: usize,
) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut output = Vec::new();
        loop {
            let batch = manager
                .poll((terminal.host_id, terminal.host_session_id, terminal.id))
                .await
                .expect("terminal poll");
            let mut batch_bytes = 0usize;
            for chunk in batch.chunks_base64 {
                let decoded = STANDARD.decode(chunk).expect("base64 output");
                assert!(
                    decoded.len() <= OUTPUT_CHUNK_BYTES,
                    "real OpenSSH output chunk exceeded the terminal limit"
                );
                batch_bytes += decoded.len();
                output.extend(decoded);
            }
            assert!(
                batch_bytes <= OUTPUT_BATCH_BYTES,
                "real OpenSSH output batch exceeded the renderer limit"
            );
            if output.windows(marker.len()).any(|window| window == marker)
                && output.len() >= minimum_bytes
            {
                return output;
            }
            assert_eq!(
                batch.session.state,
                TerminalState::Open,
                "terminal ended early"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("terminal output deadline")
}

#[tokio::test]
#[ignore = "requires tools/openssh-fixture disposable real OpenSSH server"]
async fn openssh_terminal_pty_interoperability() {
    let fixture = Fixture::from_env();
    let known_hosts = Arc::new(KnownHosts::open(&fixture.pin_db).expect("known hosts"));
    assert!(
        known_hosts
            .fingerprint(
                &fixture.host.connection.hostname,
                fixture.host.connection.port,
            )
            .expect("pin lookup")
            .is_some(),
        "Goal 01A phase A must establish trust before the terminal test"
    );
    let transport = SshProvider::new(known_hosts)
        .connect(
            &fixture.host,
            fixture.credential(),
            CancellationToken::new(),
        )
        .await
        .expect("authenticated SSH transport");
    let manager = TerminalManager::new();
    let connection = HostSessionId::new();

    let first = manager
        .open(
            fixture.host.id,
            connection,
            transport.as_ref(),
            size(90, 28),
        )
        .await
        .expect("first PTY");
    send(
        &manager,
        &first,
        b"stty -echo; printf 'NEXUS_PROMPT_READY\\n'\r",
    )
    .await;
    let prompt = collect_until(&manager, &first, b"NEXUS_PROMPT_READY", 1).await;
    assert!(!prompt.is_empty());

    let unicode = "printf 'UTF8:Здравей:✓:◆\\n'\r";
    send(&manager, &first, unicode.as_bytes()).await;
    let unicode_output = collect_until(&manager, &first, "UTF8:Здравей:✓:◆".as_bytes(), 1).await;
    assert!(
        unicode_output
            .windows("✓".len())
            .any(|window| window == "✓".as_bytes())
    );

    send(
        &manager,
        &first,
        b"printf '\x1b[31mRED\x1b[0m \x1b[1mBOLD\x1b[0m \x1b[4mUNDER\x1b[0m \x1b[7mREVERSE\x1b[0m\nANSI_DONE\n'\r",
    )
    .await;
    let ansi = collect_until(&manager, &first, b"ANSI_DONE", 1).await;
    assert!(ansi.windows(5).any(|window| window == b"\x1b[31m"));

    manager
        .resize(
            (first.host_id, first.host_session_id, first.id),
            size(101, 37),
        )
        .await
        .expect("PTY resize");
    send(&manager, &first, b"stty size; printf 'SIZE_DONE\n'\r").await;
    let resized = collect_until(&manager, &first, b"SIZE_DONE", 1).await;
    assert!(resized.windows(6).any(|window| window == b"37 101"));

    let second = manager
        .open(
            fixture.host.id,
            connection,
            transport.as_ref(),
            size(84, 26),
        )
        .await
        .expect("second PTY");
    send(&manager, &first, b"printf 'STREAM_ONE_ONLY\n'\r").await;
    send(&manager, &second, b"printf 'STREAM_TWO_ONLY\n'\r").await;
    let output_one = collect_until(&manager, &first, b"STREAM_ONE_ONLY", 1).await;
    let output_two = collect_until(&manager, &second, b"STREAM_TWO_ONLY", 1).await;
    assert!(
        !output_one
            .windows(15)
            .any(|window| window == b"STREAM_TWO_ONLY")
    );
    assert!(
        !output_two
            .windows(15)
            .any(|window| window == b"STREAM_ONE_ONLY")
    );

    manager
        .close((first.host_id, first.host_session_id, first.id))
        .await
        .expect("close first PTY");
    send(&manager, &second, b"printf 'SECOND_SURVIVED\n'\r").await;
    collect_until(&manager, &second, b"SECOND_SURVIVED", 1).await;

    send(
        &manager,
        &second,
        b"yes X | head -c 2097152; printf '\nVOLUME_DONE\n'\r",
    )
    .await;
    let volume = collect_until(&manager, &second, b"VOLUME_DONE", 2 * 1024 * 1024).await;
    assert!(volume.len() < 3 * 1024 * 1024, "bounded test capture");

    send(&manager, &second, b"top\r").await;
    let tui = collect_until(&manager, &second, b"Mem", 256).await;
    assert!(
        tui.contains(&0x1b),
        "top should emit terminal control sequences"
    );
    send(&manager, &second, b"q").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    send(&manager, &second, b"printf 'TUI_EXITED\n'\r").await;
    collect_until(&manager, &second, b"TUI_EXITED", 1).await;

    transport
        .disconnect()
        .await
        .expect("disconnect SSH transport");
    manager
        .disconnect_connection(second.host_id, second.host_session_id)
        .await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let state = manager
                .poll((second.host_id, second.host_session_id, second.id))
                .await
                .expect("poll disconnected")
                .session
                .state;
            if state == TerminalState::Disconnected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal disconnect propagation");
    manager.shutdown().await;
}
