//! Opt-in streamed SFTP acceptance against the marker-owned OpenSSH fixture.

use nexus_model::{
    AuthenticationMethod, Host, HostConnectionConfig, HostId, HostSessionId, TerminalSize,
};
use nexus_operations::RemoteSession;
use nexus_secrets::Credential;
use nexus_sftp::{
    FilePlanStore, SftpClient, SftpConnector, StagingOwnership, TransferManager, decode_text,
    join_remote,
};
use nexus_ssh::{KnownHosts, SshProvider};
use nexus_terminal::TerminalConnector;
use sha2::{Digest, Sha256};
use std::{env, fs, sync::Arc};
use tempfile::tempdir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
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

/// Pauses only the test source after a real first SFTP write acknowledgement.
/// The production client has already confirmed exclusive staging ownership.
struct PauseAfterFirstRead<'a> {
    inner: &'a mut (dyn AsyncRead + Unpin + Send),
    reads: usize,
    paused: Arc<tokio::sync::Notify>,
}
impl AsyncRead for PauseAfterFirstRead<'_> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.reads > 0 {
            self.paused.notify_one();
            return std::task::Poll::Pending;
        }
        let before = buffer.filled().len();
        let result = std::pin::Pin::new(&mut *self.inner).poll_read(cx, buffer);
        if matches!(result, std::task::Poll::Ready(Ok(()))) && buffer.filled().len() > before {
            self.reads += 1;
        }
        result
    }
}

struct PausedEditorClient {
    inner: Arc<dyn SftpClient>,
    paused: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl SftpClient for PausedEditorClient {
    fn info(&self) -> nexus_model::SftpSessionInfo {
        self.inner.info()
    }
    async fn list(
        &self,
        path: &str,
        cancel: CancellationToken,
    ) -> Result<nexus_model::DirectoryListing, nexus_model::AppError> {
        self.inner.list(path, cancel).await
    }
    async fn stat(&self, path: &str) -> Result<nexus_model::RemoteEntry, nexus_model::AppError> {
        self.inner.stat(path).await
    }
    async fn identity(
        &self,
        path: &str,
    ) -> Result<Option<nexus_sftp::EntryIdentity>, nexus_model::AppError> {
        self.inner.identity(path).await
    }
    async fn editor_revision(
        &self,
        path: &str,
    ) -> Result<nexus_sftp::EditorRevision, nexus_model::AppError> {
        self.inner.editor_revision(path).await
    }
    async fn read_editor_bytes(&self, path: &str) -> Result<Vec<u8>, nexus_model::AppError> {
        self.inner.read_editor_bytes(path).await
    }
    async fn preserve_editor_metadata(
        &self,
        staging: &str,
        expected: &nexus_sftp::EditorRevision,
    ) -> Result<(), nexus_model::AppError> {
        self.inner.preserve_editor_metadata(staging, expected).await
    }
    async fn create_dir(&self, path: &str) -> Result<(), nexus_model::AppError> {
        self.inner.create_dir(path).await
    }
    async fn remove_file(&self, path: &str) -> Result<(), nexus_model::AppError> {
        self.inner.remove_file(path).await
    }
    async fn remove_dir(&self, path: &str) -> Result<(), nexus_model::AppError> {
        self.inner.remove_dir(path).await
    }
    async fn rename_noclobber(
        &self,
        source: &str,
        destination: &str,
    ) -> Result<(), nexus_model::AppError> {
        self.inner.rename_noclobber(source, destination).await
    }
    async fn upload_staged(
        &self,
        reader: &mut (dyn AsyncRead + Unpin + Send),
        staging: &str,
        ownership: StagingOwnership,
        cancel: CancellationToken,
        progress: nexus_sftp::Progress,
    ) -> nexus_sftp::StagedUploadResult {
        let mut paused = PauseAfterFirstRead {
            inner: reader,
            reads: 0,
            paused: self.paused.clone(),
        };
        self.inner
            .upload_staged(&mut paused, staging, ownership, cancel, progress)
            .await
    }
    async fn download(
        &self,
        remote: &str,
        writer: &mut (dyn AsyncWrite + Unpin + Send),
        cancel: CancellationToken,
        progress: nexus_sftp::Progress,
    ) -> Result<u64, nexus_model::AppError> {
        self.inner.download(remote, writer, cancel, progress).await
    }
    async fn commit_new(
        &self,
        staging: &str,
        destination: &str,
    ) -> Result<(), nexus_model::AppError> {
        self.inner.commit_new(staging, destination).await
    }
    async fn commit_replace(
        &self,
        staging: &str,
        destination: &str,
    ) -> Result<(), nexus_model::AppError> {
        self.inner.commit_replace(staging, destination).await
    }
    async fn remove_owned_staging(
        &self,
        staging: &str,
    ) -> Result<nexus_sftp::StagingCleanup, nexus_model::AppError> {
        self.inner.remove_owned_staging(staging).await
    }
    async fn close(&self) {
        self.inner.close().await;
    }
}

#[tokio::test]
#[ignore = "requires tools/openssh-fixture disposable real OpenSSH server"]
async fn openssh_remote_text_editor_interoperability() {
    let fixture = Fixture::from_env();
    let known_hosts = Arc::new(KnownHosts::open(&fixture.pin_db).expect("known hosts"));
    let transport = SshProvider::new(known_hosts)
        .connect(
            &fixture.host,
            fixture.credential(),
            CancellationToken::new(),
        )
        .await
        .expect("SSH");
    let client = transport
        .open_sftp(fixture.host.id, HostSessionId::new())
        .await
        .expect("SFTP");
    let root = client.info().root_path;
    let store = FilePlanStore::default();
    let manager = TransferManager::new();
    let samples: [(&str, &[u8], &str, &[u8]); 3] = [
        ("lf", b"before\n", "after\n", b"after\n"),
        ("crlf", b"before\r\n", "after\n", b"after\r\n"),
        (
            "bom",
            b"\xef\xbb\xbfbefore\r\n",
            "after\n",
            b"\xef\xbb\xbfafter\r\n",
        ),
    ];
    for (label, original, edited, expected_bytes) in samples {
        let path = join_remote(
            &root,
            &format!("goal-02c-{label}-{}.txt", uuid::Uuid::new_v4()),
        )
        .unwrap();
        let stage =
            join_remote(&root, &format!(".nexusops-{}.part", uuid::Uuid::new_v4())).unwrap();
        let mut reader = std::io::Cursor::new(original.to_vec());
        client
            .upload_staged(
                &mut reader,
                &stage,
                StagingOwnership::new(),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .expect("initial stage");
        let staged = client
            .editor_revision(&stage)
            .await
            .expect("stage metadata");
        let mode = nexus_sftp::EditorRevision {
            raw_mode: 0o100644,
            ..staged
        };
        client
            .preserve_editor_metadata(&stage, &mode)
            .await
            .expect("initial mode");
        client
            .commit_new(&stage, &path)
            .await
            .expect("initial final");
        let before = client
            .editor_revision(&path)
            .await
            .expect("original revision");
        assert_eq!(before.raw_mode & 0o7777, 0o644);
        let bytes = client.read_editor_bytes(&path).await.expect("editor read");
        assert_eq!(bytes, original);
        let (text, newline, bom) = decode_text(&bytes).expect("text format");
        assert_eq!(text, "before\n");
        let document = store
            .register_editor_document(client.as_ref(), path.clone(), before, &bytes, newline, bom)
            .unwrap();
        let cancelled = store
            .plan_editor_save(client.clone(), document, "cancelled draft".into())
            .await
            .expect("cancelled review plan");
        assert!(
            store
                .discard(
                    cancelled.id,
                    cancelled.host_id,
                    cancelled.host_session_id,
                    cancelled.sftp_session_id
                )
                .expect("confirmed cancel")
        );
        assert!(
            store
                .consume(
                    cancelled.id,
                    cancelled.host_id,
                    cancelled.host_session_id,
                    cancelled.sftp_session_id
                )
                .is_err()
        );
        let plan = store
            .plan_editor_save(client.clone(), document, edited.into())
            .await
            .expect("plan");
        assert_ne!(plan.id, cancelled.id);
        let internal = store
            .consume(
                plan.id,
                plan.host_id,
                plan.host_session_id,
                plan.sftp_session_id,
            )
            .unwrap();
        let jobs = manager
            .execute(internal)
            .await
            .expect("enqueue editor save");
        assert_eq!(jobs.len(), 1);
        let finished = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let job = manager
                    .list(Some(fixture.host.id))
                    .into_iter()
                    .find(|job| job.id == jobs[0].id)
                    .unwrap();
                if matches!(
                    job.state,
                    nexus_model::TransferState::Completed
                        | nexus_model::TransferState::Failed
                        | nexus_model::TransferState::OutcomeUnknown
                ) {
                    return job;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("editor transfer completion");
        assert_eq!(
            finished.state,
            nexus_model::TransferState::Completed,
            "{:?}",
            finished.error
        );
        assert_eq!(
            client.read_editor_bytes(&path).await.unwrap(),
            expected_bytes
        );
        assert_eq!(
            client.editor_revision(&path).await.unwrap().raw_mode & 0o7777,
            0o644
        );

        let current = client.editor_revision(&path).await.unwrap();
        let stale_doc = store
            .register_editor_document(
                client.as_ref(),
                path.clone(),
                current,
                expected_bytes,
                newline,
                bom,
            )
            .unwrap();
        let independent =
            join_remote(&root, &format!(".nexusops-{}.part", uuid::Uuid::new_v4())).unwrap();
        let mut replacement = std::io::Cursor::new(b"other writer\n".to_vec());
        client
            .upload_staged(
                &mut replacement,
                &independent,
                StagingOwnership::new(),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        client.commit_replace(&independent, &path).await.unwrap();
        let conflict = store
            .plan_editor_save(client.clone(), stale_doc, "blind overwrite".into())
            .await
            .unwrap_err();
        assert_eq!(conflict.code, nexus_model::ErrorCode::Conflict);
        assert_eq!(
            client.read_editor_bytes(&path).await.unwrap(),
            b"other writer\n"
        );
        client.remove_file(&path).await.expect("fixture cleanup");
    }
    client.close().await;
}

#[tokio::test]
#[ignore = "requires tools/openssh-fixture disposable real OpenSSH server"]
async fn openssh_editor_disconnect_cleans_confirmed_staging() {
    let fixture = Fixture::from_env();
    let known_hosts = Arc::new(KnownHosts::open(&fixture.pin_db).expect("known hosts"));
    let transport = SshProvider::new(known_hosts)
        .connect(
            &fixture.host,
            fixture.credential(),
            CancellationToken::new(),
        )
        .await
        .expect("SSH");
    let client = transport
        .open_sftp(fixture.host.id, HostSessionId::new())
        .await
        .expect("SFTP");
    let info = client.info();
    let path = join_remote(
        &info.root_path,
        &format!("goal-02c-cancel-{}.txt", uuid::Uuid::new_v4()),
    )
    .unwrap();
    let initial_stage = join_remote(
        &info.root_path,
        &format!(".nexusops-{}.part", uuid::Uuid::new_v4()),
    )
    .unwrap();
    let original = b"original\n";
    client
        .upload_staged(
            &mut std::io::Cursor::new(original.to_vec()),
            &initial_stage,
            StagingOwnership::new(),
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    client.commit_new(&initial_stage, &path).await.unwrap();
    let revision = client.editor_revision(&path).await.unwrap();
    let (text, newline, bom) = decode_text(original).unwrap();
    assert_eq!(text, "original\n");
    let paused = Arc::new(tokio::sync::Notify::new());
    let wrapped = Arc::new(PausedEditorClient {
        inner: client.clone(),
        paused: paused.clone(),
    });
    let store = FilePlanStore::default();
    let token = store
        .register_editor_document(
            wrapped.as_ref(),
            path.clone(),
            revision,
            original,
            newline,
            bom,
        )
        .unwrap();
    let plan = store
        .plan_editor_save(
            wrapped,
            token,
            "n".repeat(nexus_model::MAX_REMOTE_EDITOR_BYTES),
        )
        .await
        .unwrap();
    let internal = store
        .consume(
            plan.id,
            plan.host_id,
            plan.host_session_id,
            plan.sftp_session_id,
        )
        .unwrap();
    let manager = TransferManager::new();
    let jobs = manager.execute(internal).await.unwrap();
    let staging = join_remote(
        &info.root_path,
        &format!(".nexusops-{}.edit.part", jobs[0].id.0.simple()),
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), paused.notified())
        .await
        .expect("first staged write acknowledged");
    assert!(client.identity(&staging).await.unwrap().is_some());
    manager.disconnect(info.host_id, info.host_session_id);
    manager
        .quiesce_session(info.host_id, info.host_session_id)
        .await
        .expect("owned staging cleanup before close");
    let finished = manager
        .list(Some(info.host_id))
        .into_iter()
        .find(|job| job.id == jobs[0].id)
        .unwrap();
    assert_eq!(finished.state, nexus_model::TransferState::Cancelled);
    assert!(client.identity(&staging).await.unwrap().is_none());
    assert_eq!(client.read_editor_bytes(&path).await.unwrap(), original);
    client.remove_file(&path).await.unwrap();
    client.close().await;
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

fn pattern_byte(offset: u64) -> u8 {
    let mixed = offset
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .rotate_left((offset % 63) as u32)
        ^ (offset >> 7)
        ^ (offset >> 23);
    (mixed ^ (mixed >> 32)) as u8
}

async fn apply_directories(
    client: Arc<dyn nexus_sftp::SftpClient>,
    paths: &[String],
    create: bool,
) {
    for batch in paths.chunks(32) {
        let mut tasks = tokio::task::JoinSet::new();
        for path in batch {
            let client = client.clone();
            let path = path.clone();
            tasks.spawn(async move {
                if create {
                    client.create_dir(&path).await
                } else {
                    client.remove_dir(&path).await
                }
            });
        }
        while let Some(result) = tasks.join_next().await {
            result
                .expect("directory task")
                .expect("directory operation");
        }
    }
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
    let original_hash = Sha256::digest(&original);
    let mut source = std::io::Cursor::new(original.clone());
    let progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let progress_copy = progress.clone();
    let written = client
        .upload_staged(
            &mut source,
            &staging,
            nexus_sftp::StagingOwnership::new(),
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
    let downloaded_original = download_bytes(client.as_ref(), &final_path).await;
    assert_eq!(Sha256::digest(&downloaded_original), original_hash);
    assert_eq!(downloaded_original, original);

    let empty = join_remote(&root, "empty").expect("empty path");
    let empty_stage = join_remote(&root, ".nexusops-empty.part").expect("empty staging");
    client
        .upload_staged(
            &mut std::io::Cursor::new(Vec::<u8>::new()),
            &empty_stage,
            nexus_sftp::StagingOwnership::new(),
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
            nexus_sftp::StagingOwnership::new(),
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
                nexus_sftp::StagingOwnership::new(),
                cancelled,
                Arc::new(|_| {})
            )
            .await
            .expect_err("cancelled upload")
            .code,
        nexus_model::ErrorCode::Cancelled
    );
    client.remove_owned_staging(&cancelled_stage).await.unwrap();

    let midstream_stage =
        join_remote(&root, ".nexusops-midstream-cancel.part").expect("cancel staging");
    let midstream_cancel = CancellationToken::new();
    let cancel_from_progress = midstream_cancel.clone();
    assert_eq!(
        client
            .upload_staged(
                &mut tokio::io::repeat(0x3c),
                &midstream_stage,
                nexus_sftp::StagingOwnership::new(),
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
    client.remove_owned_staging(&midstream_stage).await.unwrap();

    let large_bytes = 256_u64 * 1024 * 1024;
    let large = join_remote(&root, "streamed-256m.bin").expect("large path");
    let large_stage = join_remote(&root, ".nexusops-large.part").expect("large staging");
    let (mut generator_writer, mut generated) = tokio::io::duplex(128 * 1024);
    let generator = tokio::spawn(async move {
        let mut offset = 0_u64;
        let mut hash = Sha256::new();
        while offset < large_bytes {
            let uneven = 17_003 + ((offset / 65_537) % 41_113) as usize;
            let count = uneven.min((large_bytes - offset) as usize);
            let block = (0..count)
                .map(|index| pattern_byte(offset + index as u64))
                .collect::<Vec<_>>();
            generator_writer
                .write_all(&block)
                .await
                .expect("pattern source write");
            hash.update(&block);
            offset += count as u64;
        }
        drop(generator_writer);
        hash.finalize()
    });
    assert_eq!(
        client
            .upload_staged(
                &mut generated,
                &large_stage,
                nexus_sftp::StagingOwnership::new(),
                CancellationToken::new(),
                Arc::new(|_| {})
            )
            .await
            .expect("large streamed upload"),
        large_bytes
    );
    let expected_hash = generator.await.expect("pattern generator");
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
    let mut actual_hash = Sha256::new();
    let mut pattern_offset = 0_u64;
    loop {
        let count = local_file.read(&mut block).await.expect("read back");
        if count == 0 {
            break;
        }
        for (index, byte) in block[..count].iter().enumerate() {
            assert_eq!(*byte, pattern_byte(pattern_offset + index as u64));
        }
        actual_hash.update(&block[..count]);
        checked += count as u64;
        pattern_offset += count as u64;
    }
    assert_eq!(checked, large_bytes);
    assert_eq!(actual_hash.finalize(), expected_hash);

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

    let large_directory = join_remote(&root, "large-listing").expect("large directory");
    client
        .create_dir(&large_directory)
        .await
        .expect("large mkdir");
    let large_children = (0..=nexus_sftp::LISTING_ENTRY_CAP)
        .map(|index| join_remote(&large_directory, &format!("entry-{index:05}")).unwrap())
        .collect::<Vec<_>>();
    apply_directories(client.clone(), &large_children, true).await;
    let capped = client
        .list(&large_directory, CancellationToken::new())
        .await
        .expect("capped real listing");
    assert_eq!(capped.entries.len(), 5_000);
    assert!(capped.partial);
    apply_directories(client.clone(), &large_children, false).await;
    client
        .remove_dir(&large_directory)
        .await
        .expect("large rmdir");

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
