use crate::path::{display_name, join_remote, validate_child_name, validate_remote_path};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexus_model::{
    AppError, DirectoryListing, ErrorCode, HostId, HostSessionId, RemoteEntry, RemoteEntryKind,
    SftpExtension, SftpLimits, SftpSessionId, SftpSessionInfo,
};
use russh_sftp::{
    client::{Config, RawSftpSession, error::Error as SftpError},
    protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;

pub const CLIENT_CHUNK_BYTES: usize = 64 * 1024;
pub const LISTING_ENTRY_CAP: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryIdentity {
    pub kind: RemoteEntryKind,
    pub size: Option<u64>,
    pub modified: Option<u32>,
}

pub type Progress = Arc<dyn Fn(u64) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum StagedUploadFailure {
    NotCreated(AppError),
    OwnedFailure(AppError),
    CreationOutcomeUnknown(AppError),
}
pub type StagedUploadResult = Result<u64, StagedUploadFailure>;

impl std::ops::Deref for StagedUploadFailure {
    type Target = AppError;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::NotCreated(error)
            | Self::OwnedFailure(error)
            | Self::CreationOutcomeUnknown(error) => error,
        }
    }
}

#[derive(Clone, Default)]
pub struct StagingOwnership(Arc<AtomicBool>);

impl StagingOwnership {
    pub fn new() -> Self {
        Self::default()
    }
    /// Confirms that the server created the exact exclusive staging path.
    ///
    /// Implementations of [`SftpClient::upload_staged`] must call this as soon
    /// as exclusive creation succeeds, before streaming any bytes.
    pub fn mark_owned(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_owned(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[async_trait]
pub trait SftpClient: Send + Sync {
    fn info(&self) -> SftpSessionInfo;
    async fn list(
        &self,
        path: &str,
        cancel: CancellationToken,
    ) -> Result<DirectoryListing, AppError>;
    async fn stat(&self, path: &str) -> Result<RemoteEntry, AppError>;
    async fn identity(&self, path: &str) -> Result<Option<EntryIdentity>, AppError>;
    async fn create_dir(&self, path: &str) -> Result<(), AppError>;
    async fn remove_file(&self, path: &str) -> Result<(), AppError>;
    async fn remove_dir(&self, path: &str) -> Result<(), AppError>;
    async fn rename_noclobber(&self, source: &str, destination: &str) -> Result<(), AppError>;
    async fn upload_staged(
        &self,
        reader: &mut (dyn AsyncRead + Unpin + Send),
        staging: &str,
        ownership: StagingOwnership,
        cancel: CancellationToken,
        progress: Progress,
    ) -> StagedUploadResult;
    async fn download(
        &self,
        remote: &str,
        writer: &mut (dyn AsyncWrite + Unpin + Send),
        cancel: CancellationToken,
        progress: Progress,
    ) -> Result<u64, AppError>;
    async fn commit_new(&self, staging: &str, destination: &str) -> Result<(), AppError>;
    async fn commit_replace(&self, staging: &str, destination: &str) -> Result<(), AppError>;
    async fn remove_owned_staging(&self, staging: &str);
    async fn close(&self);
}

pub struct RawSftpClient {
    raw: RawSftpSession,
    info: SftpSessionInfo,
    extension_map: HashMap<String, String>,
    read_chunk: u32,
    write_chunk: usize,
}

impl RawSftpClient {
    pub async fn connect<S>(
        stream: S,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<Arc<dyn SftpClient>, AppError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let config = Config {
            max_packet_len: 256 * 1024,
            max_concurrent_reads: 4,
            max_concurrent_writes: 4,
            max_write_packet_len: CLIENT_CHUNK_BYTES as u32,
            request_timeout_secs: 15,
        };
        let mut raw = RawSftpSession::new_with_config(stream, config);
        let version = raw.init().await.map_err(initialization_error)?;
        if version.version != 3 {
            let _ = raw.close_session();
            return Err(AppError::new(
                ErrorCode::SftpUnavailable,
                "The server negotiated an unsupported SFTP protocol version.",
            ));
        }
        let extension_map = version.extensions;
        let negotiated = if extension_map
            .get("limits@openssh.com")
            .is_some_and(|v| v == "1")
        {
            let value = raw.limits().await.map_err(initialization_error)?;
            let limits = russh_sftp::client::rawsession::Limits {
                packet_len: (value.max_packet_len > 0).then_some(value.max_packet_len),
                read_len: (value.max_read_len > 0).then_some(value.max_read_len),
                write_len: (value.max_write_len > 0).then_some(value.max_write_len),
                open_handles: (value.max_open_handles > 0).then_some(value.max_open_handles),
            };
            raw.set_limits(limits);
            Some(limits)
        } else {
            None
        };
        let root = raw
            .realpath(".")
            .await
            .map_err(map_error)?
            .files
            .into_iter()
            .next()
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::SftpProtocol,
                    "The SFTP server returned no start directory.",
                )
            })?
            .filename;
        validate_remote_path(&root)?;
        let read_chunk = negotiated
            .and_then(|v| v.read_len)
            .unwrap_or(CLIENT_CHUNK_BYTES as u64)
            .min(CLIENT_CHUNK_BYTES as u64)
            .max(1) as u32;
        let write_chunk = negotiated
            .and_then(|v| v.write_len)
            .unwrap_or(CLIENT_CHUNK_BYTES as u64)
            .min(CLIENT_CHUNK_BYTES as u64)
            .max(1) as usize;
        let limits = SftpLimits {
            max_packet_bytes: negotiated.and_then(|v| v.packet_len).map(|v| v.to_string()),
            max_read_bytes: negotiated.and_then(|v| v.read_len).map(|v| v.to_string()),
            max_write_bytes: negotiated.and_then(|v| v.write_len).map(|v| v.to_string()),
            max_open_handles: negotiated
                .and_then(|v| v.open_handles)
                .map(|v| v.to_string()),
            client_chunk_bytes: CLIENT_CHUNK_BYTES.to_string(),
            listing_entry_cap: LISTING_ENTRY_CAP as u32,
        };
        let mut extensions = extension_map
            .iter()
            .map(|(name, version)| SftpExtension {
                name: name.clone(),
                version: version.clone(),
            })
            .collect::<Vec<_>>();
        extensions.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Arc::new(Self {
            raw,
            info: SftpSessionInfo {
                id: SftpSessionId::new(),
                host_id,
                host_session_id,
                protocol_version: 3,
                extensions,
                limits,
                root_path: root,
            },
            extension_map,
            read_chunk,
            write_chunk,
        }))
    }

    fn supports(&self, name: &str, version: &str) -> bool {
        self.extension_map
            .get(name)
            .is_some_and(|value| value == version)
    }

    async fn status_ok(
        result: Result<russh_sftp::protocol::Status, SftpError>,
    ) -> Result<(), AppError> {
        let status = result.map_err(map_error)?;
        if status.status_code == StatusCode::Ok {
            Ok(())
        } else {
            Err(map_status(status.status_code))
        }
    }

    async fn mutation_status(
        result: Result<russh_sftp::protocol::Status, SftpError>,
        unknown_message: &'static str,
    ) -> Result<(), AppError> {
        match result {
            Ok(status) if status.status_code == StatusCode::Ok => Ok(()),
            Ok(status) => Err(map_status(status.status_code)),
            Err(SftpError::Status(status)) => Err(map_status(status.status_code)),
            Err(_) => Err(AppError::new(ErrorCode::OutcomeUnknown, unknown_message)),
        }
    }

    async fn close_handle(&self, handle: String) -> Result<(), AppError> {
        Self::status_ok(self.raw.close(handle).await).await
    }
}

#[async_trait]
impl SftpClient for RawSftpClient {
    fn info(&self) -> SftpSessionInfo {
        self.info.clone()
    }

    async fn list(
        &self,
        path: &str,
        cancel: CancellationToken,
    ) -> Result<DirectoryListing, AppError> {
        validate_remote_path(path)?;
        let handle = self.raw.opendir(path).await.map_err(map_error)?.handle;
        let mut entries = Vec::new();
        let mut partial = false;
        let result = 'read_directory: loop {
            let response = tokio::select! {
                biased;
                _ = cancel.cancelled() => break Err(AppError::new(ErrorCode::Cancelled, "Directory loading was cancelled.")),
                value = self.raw.readdir(handle.clone()) => value,
            };
            match response {
                Ok(batch) => {
                    for file in batch.files {
                        if file.filename == "." || file.filename == ".." {
                            continue;
                        }
                        if let Err(error) = validate_child_name(&file.filename) {
                            break 'read_directory Err(error);
                        }
                        if entries.len() == LISTING_ENTRY_CAP {
                            partial = true;
                            break;
                        }
                        match remote_entry(path, file.filename, file.attrs) {
                            Ok(entry) => entries.push(entry),
                            Err(error) => break 'read_directory Err(error),
                        }
                    }
                    if partial {
                        break Ok(());
                    }
                }
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                    break Ok(());
                }
                Err(error) => break Err(map_error(error)),
            }
        };
        let close = self.close_handle(handle).await;
        result?;
        close?;
        Ok(DirectoryListing {
            host_id: self.info.host_id,
            host_session_id: self.info.host_session_id,
            sftp_session_id: self.info.id,
            path: path.into(),
            entries,
            partial,
            entry_cap: LISTING_ENTRY_CAP as u32,
        })
    }

    async fn stat(&self, path: &str) -> Result<RemoteEntry, AppError> {
        validate_remote_path(path)?;
        let attrs = self.raw.lstat(path).await.map_err(map_error)?.attrs;
        let name = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .to_owned();
        remote_entry(path.rsplit_once('/').map_or("", |v| v.0), name, attrs)
    }

    async fn identity(&self, path: &str) -> Result<Option<EntryIdentity>, AppError> {
        validate_remote_path(path)?;
        match self.raw.lstat(path).await {
            Ok(value) => Ok(Some(identity(&value.attrs))),
            Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                Ok(None)
            }
            Err(error) => Err(map_error(error)),
        }
    }

    async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        validate_remote_path(path)?;
        if self.identity(path).await?.is_some() {
            return Err(conflict());
        }
        let attrs = FileAttributes {
            permissions: Some(0o040700),
            ..FileAttributes::default()
        };
        Self::mutation_status(
            self.raw.mkdir(path, attrs).await,
            "The create-directory reply was lost; refresh before taking another action.",
        )
        .await
    }

    async fn remove_file(&self, path: &str) -> Result<(), AppError> {
        validate_remote_path(path)?;
        let entry = self.stat(path).await?;
        if !matches!(entry.kind, RemoteEntryKind::File | RemoteEntryKind::Symlink) {
            return Err(AppError::new(
                ErrorCode::FilePolicy,
                "Only regular files and symbolic links can be deleted as files.",
            ));
        }
        Self::mutation_status(
            self.raw.remove(path).await,
            "The delete reply was lost; refresh before taking another action.",
        )
        .await
    }

    async fn remove_dir(&self, path: &str) -> Result<(), AppError> {
        validate_remote_path(path)?;
        if self.stat(path).await?.kind != RemoteEntryKind::Directory {
            return Err(AppError::new(
                ErrorCode::FilePolicy,
                "Only a directory can be removed with this action.",
            ));
        }
        Self::mutation_status(
            self.raw.rmdir(path).await,
            "The remove-directory reply was lost; refresh before taking another action.",
        )
        .await
    }

    async fn rename_noclobber(&self, source: &str, destination: &str) -> Result<(), AppError> {
        validate_remote_path(source)?;
        validate_remote_path(destination)?;
        if crate::path::parent_remote(source)? != crate::path::parent_remote(destination)? {
            return Err(AppError::new(
                ErrorCode::FilePolicy,
                "Rename is limited to the same remote directory.",
            ));
        }
        let source_identity = self.identity(source).await?.ok_or_else(not_found)?;
        if source_identity.kind != RemoteEntryKind::File
            || !self.supports("hardlink@openssh.com", "1")
        {
            return Err(AppError::new(
                ErrorCode::SftpUnavailable,
                "Safe no-clobber rename is available only for regular files when hardlink@openssh.com v1 is negotiated.",
            ));
        }
        if self.identity(destination).await?.is_some() {
            return Err(conflict());
        }
        Self::mutation_status(
            self.raw.hardlink(source, destination).await,
            "The rename reply was lost; refresh before taking another action.",
        )
        .await?;
        match Self::mutation_status(
            self.raw.remove(source).await,
            "The new name exists, but removal of the old name could not be confirmed.",
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(_) => Err(AppError::new(
                ErrorCode::OutcomeUnknown,
                "The new name was created, but removal of the old name could not be confirmed.",
            )),
        }
    }

    async fn upload_staged(
        &self,
        reader: &mut (dyn AsyncRead + Unpin + Send),
        staging: &str,
        ownership: StagingOwnership,
        cancel: CancellationToken,
        progress: Progress,
    ) -> StagedUploadResult {
        if let Err(error) = validate_remote_path(staging) {
            return Err(StagedUploadFailure::NotCreated(error));
        }
        let attrs = FileAttributes {
            permissions: Some(0o100600),
            ..FileAttributes::default()
        };
        let handle = match self
            .raw
            .open(
                staging,
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                attrs,
            )
            .await
        {
            Ok(value) => value.handle,
            Err(SftpError::Status(status)) => {
                return Err(StagedUploadFailure::NotCreated(map_status(
                    status.status_code,
                )));
            }
            Err(_) => {
                return Err(StagedUploadFailure::CreationOutcomeUnknown(AppError::new(
                    ErrorCode::OutcomeUnknown,
                    "The staging-file create reply was lost; ownership cannot be established.",
                )));
            }
        };
        ownership.mark_owned();
        let mut confirmed = 0u64;
        let mut buffer = vec![0u8; self.write_chunk];
        let stream_result = async {
            loop {
                let read = tokio::select! { biased; _ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled, "The transfer was cancelled.")), value = tokio::io::AsyncReadExt::read(reader, &mut buffer) => value.map_err(local_io)? };
                if read == 0 { break; }
                Self::status_ok(self.raw.write(handle.clone(), confirmed, buffer[..read].to_vec()).await).await?;
                confirmed = confirmed.checked_add(read as u64).ok_or_else(|| AppError::new(ErrorCode::Transfer, "The transfer offset overflowed."))?;
                progress(confirmed);
            }
            if self.supports("fsync@openssh.com", "1") { Self::status_ok(self.raw.fsync(handle.clone()).await).await?; }
            Ok(confirmed)
        }.await;
        let close = self.close_handle(handle).await;
        match (stream_result, close) {
            (Ok(_), Ok(())) => Ok(confirmed),
            (Err(error), _) | (Ok(_), Err(error)) => Err(StagedUploadFailure::OwnedFailure(error)),
        }
    }

    async fn download(
        &self,
        remote: &str,
        writer: &mut (dyn AsyncWrite + Unpin + Send),
        cancel: CancellationToken,
        progress: Progress,
    ) -> Result<u64, AppError> {
        validate_remote_path(remote)?;
        if self.stat(remote).await?.kind != RemoteEntryKind::File {
            return Err(AppError::new(
                ErrorCode::FilePolicy,
                "Only regular files can be downloaded.",
            ));
        }
        let handle = self
            .raw
            .open(remote, OpenFlags::READ, FileAttributes::default())
            .await
            .map_err(map_error)?
            .handle;
        let mut confirmed = 0u64;
        let stream_result = async {
            loop {
                let response = tokio::select! { biased; _ = cancel.cancelled() => break Err(AppError::new(ErrorCode::Cancelled, "The transfer was cancelled.")), value = self.raw.read(handle.clone(), confirmed, self.read_chunk) => value };
                match response {
                    Ok(data) if data.data.is_empty() => break Ok(confirmed),
                    Ok(data) => {
                        tokio::io::AsyncWriteExt::write_all(writer, &data.data)
                            .await
                            .map_err(local_io)?;
                        confirmed = confirmed
                            .checked_add(data.data.len() as u64)
                            .ok_or_else(|| {
                                AppError::new(
                                    ErrorCode::Transfer,
                                    "The transfer offset overflowed.",
                                )
                            })?;
                        progress(confirmed);
                    }
                    Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                        break Ok(confirmed);
                    }
                    Err(error) => break Err(map_error(error)),
                }
            }
        }
        .await;
        let close = self.close_handle(handle).await;
        let bytes = stream_result?;
        close?;
        tokio::io::AsyncWriteExt::flush(writer)
            .await
            .map_err(local_io)?;
        Ok(bytes)
    }

    async fn commit_new(&self, staging: &str, destination: &str) -> Result<(), AppError> {
        if !self.supports("hardlink@openssh.com", "1") {
            return Err(AppError::new(
                ErrorCode::SftpUnavailable,
                "Safe no-clobber finalization requires hardlink@openssh.com v1.",
            ));
        }
        if self.identity(destination).await?.is_some() {
            return Err(conflict());
        }
        Self::mutation_status(
            self.raw.hardlink(staging, destination).await,
            "The new-file commit reply was lost; refresh before taking another action.",
        )
        .await?;
        match Self::mutation_status(
            self.raw.remove(staging).await,
            "The final file exists, but staging cleanup could not be confirmed.",
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(_) => Err(AppError::new(
                ErrorCode::OutcomeUnknown,
                "The final file exists, but staging cleanup could not be confirmed.",
            )),
        }
    }

    async fn commit_replace(&self, staging: &str, destination: &str) -> Result<(), AppError> {
        if !self.supports("posix-rename@openssh.com", "1") {
            return Err(AppError::new(
                ErrorCode::SftpUnavailable,
                "Safe Replace requires posix-rename@openssh.com v1.",
            ));
        }
        let mut data = Vec::with_capacity(staging.len() + destination.len() + 8);
        encode_string(&mut data, staging)?;
        encode_string(&mut data, destination)?;
        match self.raw.extended("posix-rename@openssh.com", data).await {
            Ok(Packet::Status(status)) if status.status_code == StatusCode::Ok => Ok(()),
            Ok(Packet::Status(status)) => Err(map_status(status.status_code)),
            Ok(_) | Err(_) => Err(AppError::new(
                ErrorCode::OutcomeUnknown,
                "The Replace reply could not confirm the final destination; refresh before taking another action.",
            )),
        }
    }

    async fn remove_owned_staging(&self, staging: &str) {
        let _ = self.raw.remove(staging).await;
    }
    async fn close(&self) {
        let _ = self.raw.close_session();
    }
}

fn remote_entry(
    parent: &str,
    name: String,
    attrs: FileAttributes,
) -> Result<RemoteEntry, AppError> {
    validate_child_name(&name)?;
    let path = if parent.is_empty() {
        name.clone()
    } else {
        join_remote(parent, &name)?
    };
    let kind = identity(&attrs).kind;
    let modified_at = attrs
        .mtime
        .and_then(|value| DateTime::<Utc>::from_timestamp(value as i64, 0))
        .map(|value| value.to_rfc3339());
    Ok(RemoteEntry {
        display_name: display_name(&name),
        name,
        path,
        kind,
        size_bytes: attrs.size.map(|v| v.to_string()),
        modified_at,
        permissions: attrs.permissions.map(|_| attrs.permissions().to_string()),
        uid: attrs.uid,
        gid: attrs.gid,
    })
}

fn identity(attrs: &FileAttributes) -> EntryIdentity {
    let kind = if attrs.is_regular() {
        RemoteEntryKind::File
    } else if attrs.is_dir() {
        RemoteEntryKind::Directory
    } else if attrs.is_symlink() {
        RemoteEntryKind::Symlink
    } else if attrs.permissions.is_none() {
        RemoteEntryKind::Unknown
    } else {
        RemoteEntryKind::Other
    };
    EntryIdentity {
        kind,
        size: attrs.size,
        modified: attrs.mtime,
    }
}

fn initialization_error(error: SftpError) -> AppError {
    match error {
        SftpError::Status(status) if status.status_code == StatusCode::PermissionDenied => {
            AppError::new(
                ErrorCode::SftpDenied,
                "The SSH server denied the SFTP subsystem.",
            )
        }
        _ => AppError::new(
            ErrorCode::SftpUnavailable,
            "The SSH server did not provide a usable SFTP subsystem.",
        ),
    }
}
fn map_error(error: SftpError) -> AppError {
    match error {
        SftpError::Status(status) => map_status(status.status_code),
        SftpError::Timeout => AppError::new(
            ErrorCode::Timeout,
            "The SFTP server did not respond in time.",
        ),
        SftpError::Limited(_) => AppError::new(
            ErrorCode::SftpProtocol,
            "The SFTP request exceeded a negotiated limit.",
        ),
        _ => AppError::new(ErrorCode::SftpProtocol, "The SFTP request failed."),
    }
}
fn map_status(status: StatusCode) -> AppError {
    match status {
        StatusCode::NoSuchFile => not_found(),
        StatusCode::PermissionDenied => AppError::new(
            ErrorCode::SftpDenied,
            "The remote account denied this file operation.",
        ),
        StatusCode::OpUnsupported => AppError::new(
            ErrorCode::SftpUnavailable,
            "The SFTP server does not support this operation.",
        ),
        StatusCode::NoConnection | StatusCode::ConnectionLost => {
            AppError::new(ErrorCode::Connection, "The SFTP connection was lost.")
        }
        _ => AppError::new(
            ErrorCode::SftpProtocol,
            "The SFTP server rejected the operation.",
        ),
    }
}
fn not_found() -> AppError {
    AppError::new(ErrorCode::NotFound, "The remote entry no longer exists.")
}
fn conflict() -> AppError {
    AppError::new(ErrorCode::Conflict, "The destination already exists.")
}
fn local_io(_: std::io::Error) -> AppError {
    AppError::new(
        ErrorCode::LocalAccess,
        "The selected local file could not be read or written.",
    )
}
fn encode_string(output: &mut Vec<u8>, value: &str) -> Result<(), AppError> {
    let len = u32::try_from(value.len())
        .map_err(|_| AppError::new(ErrorCode::FilePolicy, "The remote path is too long."))?;
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}
