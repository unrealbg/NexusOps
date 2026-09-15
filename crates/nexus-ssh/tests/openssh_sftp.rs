//! Opt-in streamed SFTP acceptance against the marker-owned OpenSSH fixture.

use nexus_model::{
    AuthenticationMethod, Host, HostConnectionConfig, HostId, HostSessionId, TerminalSize,
};
use nexus_operations::RemoteSession;
use nexus_secrets::Credential;
use nexus_sftp::{SftpConnector, join_remote};
use nexus_ssh::{KnownHosts, SshProvider};
use nexus_terminal::TerminalConnector;
use std::{env, fs, sync::Arc};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
                display_name: "Disposable OpenSSH SFTP fixture".into(),
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

async fn download_bytes(client: &dyn nexus_sftp::SftpClient, path: &str) -> Vec<u8> {
    let mut output = Vec::new();
    client
        .download(
            path,
            &mut output,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .expect("download");
    output
}

#[tokio::test]
#[ignore = "requires tools/openssh-fixture disposable real OpenSSH server"]
async fn openssh_sftp_streaming_interoperability() {
    let fixture = Fixture::from_env();
    let known_hosts = Arc::new(KnownHosts::open(&fixture.pin_db).expect("known hosts"));
    assert!(
        known_hosts
            .fingerprint(
                &fixture.host.connection.hostname,
                fixture.host.connection.port
            )
            .expect("pin lookup")
            .is_some(),
        "phase A must establish trust first"
    );
    let transport = SshProvider::new(known_hosts)
        .connect(
            &fixture.host,
            fixture.credential(),
            CancellationToken::new(),
        )
        .await
        .expect("authenticated SSH transport");
    let connection = HostSessionId::new();
    let client = transport
        .open_sftp(fixture.host.id, connection)
        .await
        .expect("real SFTP subsystem");
    let info = client.info();
    assert_eq!(info.protocol_version, 3);
    assert!(
        info.extensions
            .iter()
            .any(|extension| extension.name == "hardlink@openssh.com" && extension.version == "1")
    );
    assert!(
        info.extensions
            .iter()
            .any(|extension| extension.name == "posix-rename@openssh.com"
                && extension.version == "1")
    );
    assert_eq!(info.limits.client_chunk_bytes, "65536");

    let root = join_remote(
        &info.root_path,
        &format!("nexusops-sftp-{}", uuid::Uuid::new_v4().simple()),
    )
    .expect("owned root");
    client.create_dir(&root).await.expect("mkdir");
    let listing = client
        .list(&info.root_path, CancellationToken::new())
        .await
        .expect("browse");
    assert!(listing.entries.iter().any(|entry| entry.path == root));
    assert_eq!(
        client.stat(&root).await.expect("stat").kind,
        nexus_model::RemoteEntryKind::Directory
    );

    let name = "кирилица ' -$[];.bin";
    let final_path = join_remote(&root, name).expect("unicode path");
    let staging = join_remote(&root, ".nexusops-small.part").expect("staging");
    let original = b"\0\xffbinary\ncontent\r\n".to_vec();
    let mut source = std::io::Cursor::new(original.clone());
    let progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let progress_copy = progress.clone();
    let written = client
        .upload_staged(
            &mut source,
            &staging,
            CancellationToken::new(),
            Arc::new(move |value| {
                progress_copy.store(value, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .await
        .expect("stream upload");
    assert_eq!(written, original.len() as u64);
    assert_eq!(progress.load(std::sync::atomic::Ordering::SeqCst), written);
    client
        .commit_new(&staging, &final_path)
        .await
        .expect("no-clobber finalize");
    assert_eq!(download_bytes(client.as_ref(), &final_path).await, original);

    let empty = join_remote(&root, "empty").expect("empty path");
    let empty_stage = join_remote(&root, ".nexusops-empty.part").expect("empty staging");
    client
        .upload_staged(
            &mut std::io::Cursor::new(Vec::<u8>::new()),
            &empty_stage,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .expect("empty upload");
    client
        .commit_new(&empty_stage, &empty)
        .await
        .expect("empty finalize");
    assert!(download_bytes(client.as_ref(), &empty).await.is_empty());

    let conflicting_stage =
        join_remote(&root, ".nexusops-conflict.part").expect("conflict staging");
    client
        .upload_staged(
            &mut std::io::Cursor::new(b"replacement".to_vec()),
            &conflicting_stage,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .expect("conflict stage");
    assert!(
        client
            .commit_new(&conflicting_stage, &final_path)
            .await
            .is_err()
    );
    assert_eq!(
        download_bytes(client.as_ref(), &final_path).await,
        original,
        "no-clobber preserves old destination"
    );
    client
        .commit_replace(&conflicting_stage, &final_path)
        .await
        .expect("safe OpenSSH replace");
    assert_eq!(
        download_bytes(client.as_ref(), &final_path).await,
        b"replacement"
    );

    let renamed = join_remote(&root, "renamed.bin").expect("rename path");
    client
        .rename_noclobber(&final_path, &renamed)
        .await
        .expect("no-clobber rename");
    assert!(
        client
            .identity(&final_path)
            .await
            .expect("old identity")
            .is_none()
    );
    assert!(
        client
            .identity(&renamed)
            .await
            .expect("new identity")
            .is_some()
    );

    let cancelled_stage = join_remote(&root, ".nexusops-cancelled.part").expect("cancel staging");
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        client
            .upload_staged(
                &mut tokio::io::repeat(0x5a),
                &cancelled_stage,
                cancelled,
                Arc::new(|_| {})
            )
            .await
            .expect_err("cancelled upload")
            .code,
        nexus_model::ErrorCode::Cancelled
    );
    client.remove_owned_staging(&cancelled_stage).await;

    let midstream_stage =
        join_remote(&root, ".nexusops-midstream-cancel.part").expect("cancel staging");
    let midstream_cancel = CancellationToken::new();
    let cancel_from_progress = midstream_cancel.clone();
    assert_eq!(
        client
            .upload_staged(
                &mut tokio::io::repeat(0x3c),
                &midstream_stage,
                midstream_cancel,
                Arc::new(move |confirmed| {
                    if confirmed >= 1024 * 1024 {
                        cancel_from_progress.cancel();
                    }
                })
            )
            .await
            .expect_err("mid-stream cancelled upload")
            .code,
        nexus_model::ErrorCode::Cancelled
    );
    client.remove_owned_staging(&midstream_stage).await;

    let large_bytes = 256_u64 * 1024 * 1024;
    let large = join_remote(&root, "streamed-256m.bin").expect("large path");
    let large_stage = join_remote(&root, ".nexusops-large.part").expect("large staging");
    let mut generated = tokio::io::repeat(0xa5).take(large_bytes);
    assert_eq!(
        client
            .upload_staged(
                &mut generated,
                &large_stage,
                CancellationToken::new(),
                Arc::new(|_| {})
            )
            .await
            .expect("large streamed upload"),
        large_bytes
    );
    client
        .commit_new(&large_stage, &large)
        .await
        .expect("large finalize");
    let local = tempdir().expect("local temp");
    let local_path = local.path().join("streamed-256m.bin");
    let mut local_file = tokio::fs::File::create(&local_path)
        .await
        .expect("local file");
    assert_eq!(
        client
            .download(
                &large,
                &mut local_file,
                CancellationToken::new(),
                Arc::new(|_| {})
            )
            .await
            .expect("large streamed download"),
        large_bytes
    );
    local_file.flush().await.expect("flush");
    drop(local_file);
    let mut local_file = tokio::fs::File::open(&local_path)
        .await
        .expect("reopen for verification");
    let mut checked = 0_u64;
    let mut block = vec![0_u8; 64 * 1024];
    loop {
        let count = local_file.read(&mut block).await.expect("read back");
        if count == 0 {
            break;
        }
        assert!(block[..count].iter().all(|byte| *byte == 0xa5));
        checked += count as u64;
    }
    assert_eq!(checked, large_bytes);

    let terminal = transport
        .open_terminal(
            TerminalSize {
                columns: 80,
                rows: 24,
                pixel_width: 0,
                pixel_height: 0,
            },
            CancellationToken::new(),
        )
        .await
        .expect("terminal beside SFTP");
    assert!(
        !client
            .list(&root, CancellationToken::new())
            .await
            .expect("SFTP while PTY open")
            .entries
            .is_empty()
    );
    terminal.close().await.expect("terminal close");

    for path in [&renamed, &empty, &large] {
        client.remove_file(path).await.expect("remove file");
    }
    client
        .remove_dir(&root)
        .await
        .expect("remove empty directory");
    client.close().await;
    transport.disconnect().await.expect("SSH disconnect");
    assert!(!fs::exists(local_path).expect("local evidence metadata") || checked == large_bytes);
}
