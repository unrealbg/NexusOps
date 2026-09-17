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

#[cfg(test)]
mod fault_tests {
    use super::*;
    use russh_sftp::{
        protocol::{Attrs, Data, File, Handle, Name, Status, Version},
        server::{self, Handler},
    };
    use std::{
        io,
        pin::Pin,
        sync::Mutex,
        task::{Context, Poll},
    };

    #[derive(Default)]
    struct Trace {
        close: usize,
        reads: usize,
    }

    struct Fixture(Arc<Mutex<Trace>>);

    impl Handler for Fixture {
        type Error = StatusCode;

        fn unimplemented(&self) -> Self::Error {
            StatusCode::OpUnsupported
        }

        async fn init(
            &mut self,
            _: u32,
            _: HashMap<String, String>,
        ) -> Result<Version, Self::Error> {
            Ok(Version::new())
        }

        async fn realpath(&mut self, id: u32, _: String) -> Result<Name, Self::Error> {
            Ok(Name {
                id,
                files: vec![File::dummy("/fixture")],
            })
        }

        async fn lstat(&mut self, id: u32, _: String) -> Result<Attrs, Self::Error> {
            Ok(Attrs {
                id,
                attrs: FileAttributes {
                    permissions: Some(0o100600),
                    size: Some(12),
                    ..FileAttributes::default()
                },
            })
        }

        async fn open(
            &mut self,
            id: u32,
            _: String,
            _: OpenFlags,
            _: FileAttributes,
        ) -> Result<Handle, Self::Error> {
            Ok(Handle {
                id,
                handle: "fixture-handle".into(),
            })
        }

        async fn read(&mut self, id: u32, _: String, _: u64, _: u32) -> Result<Data, Self::Error> {
            let mut trace = self.0.lock().unwrap();
            trace.reads += 1;
            Ok(Data {
                id,
                data: if trace.reads == 1 {
                    b"first-".to_vec()
                } else if trace.reads == 2 {
                    b"second".to_vec()
                } else {
                    return Err(StatusCode::Eof);
                },
            })
        }

        async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
            self.0.lock().unwrap().close += 1;
            Ok(Status {
                id,
                status_code: StatusCode::Ok,
                error_message: String::new(),
                language_tag: String::new(),
            })
        }
    }

    struct FullWriter {
        accepted: usize,
    }

    impl AsyncWrite for FullWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.accepted >= 6 {
                Poll::Ready(Err(io::Error::from(io::ErrorKind::StorageFull)))
            } else {
                let count = buf.len().min(6 - self.accepted);
                self.accepted += count;
                Poll::Ready(Ok(count))
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn download_closes_remote_handle_after_partial_local_disk_full() {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let (client_stream, server_stream) = tokio::io::duplex(128 * 1024);
        server::run(server_stream, Fixture(trace.clone())).await;
        let client = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            RawSftpClient::connect(client_stream, HostId::new(), HostSessionId::new()),
        )
        .await
        .unwrap()
        .unwrap();
        let mut writer = FullWriter { accepted: 0 };
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_clone = progress.clone();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.download(
                "/fixture/source",
                &mut writer,
                CancellationToken::new(),
                Arc::new(move |bytes| progress_clone.lock().unwrap().push(bytes)),
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, ErrorCode::LocalAccess);
        assert_eq!(writer.accepted, 6);
        assert_eq!(*progress.lock().unwrap(), vec![6]);
        assert_eq!(trace.lock().unwrap().close, 1);
        assert_eq!(trace.lock().unwrap().reads, 2);
    }

    struct FlushFailWriter(Vec<u8>);

    impl AsyncWrite for FlushFailWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.0.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::from(io::ErrorKind::StorageFull)))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn download_flush_failure_after_all_bytes_is_not_success() {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let (client_stream, server_stream) = tokio::io::duplex(128 * 1024);
        server::run(server_stream, Fixture(trace.clone())).await;
        let client = RawSftpClient::connect(client_stream, HostId::new(), HostSessionId::new())
            .await
            .unwrap();
        let mut writer = FlushFailWriter(Vec::new());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.download(
                "/fixture/source",
                &mut writer,
                CancellationToken::new(),
                Arc::new(|_| {}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, ErrorCode::LocalAccess);
        assert_eq!(writer.0, b"first-second");
        assert_eq!(trace.lock().unwrap().close, 1);
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        OpenDenied,
        WriteDenied,
        ReadDenied,
        FsyncDenied,
        CloseDenied,
        CommitDenied,
        CommitRace,
        CleanupDenied,
    }

    struct MatrixState {
        fault: Fault,
        files: HashMap<String, Vec<u8>>,
        calls: Vec<String>,
        writes: usize,
        reads: usize,
    }

    struct MatrixFixture(Arc<Mutex<MatrixState>>);

    fn ok(id: u32) -> Status {
        Status {
            id,
            status_code: StatusCode::Ok,
            error_message: String::new(),
            language_tag: String::new(),
        }
    }

    impl Handler for MatrixFixture {
        type Error = StatusCode;

        fn unimplemented(&self) -> Self::Error {
            StatusCode::OpUnsupported
        }

        async fn init(
            &mut self,
            _: u32,
            _: HashMap<String, String>,
        ) -> Result<Version, Self::Error> {
            let mut version = Version::new();
            version
                .extensions
                .insert("fsync@openssh.com".into(), "1".into());
            version
                .extensions
                .insert("hardlink@openssh.com".into(), "1".into());
            version
                .extensions
                .insert("posix-rename@openssh.com".into(), "1".into());
            Ok(version)
        }

        async fn realpath(&mut self, id: u32, _: String) -> Result<Name, Self::Error> {
            Ok(Name {
                id,
                files: vec![File::dummy("/fixture")],
            })
        }

        async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("LSTAT {path}"));
            if state.fault == Fault::CommitRace && path == "/fixture/new-final" {
                state.files.insert(path, b"competitor".to_vec());
                return Err(StatusCode::NoSuchFile);
            }
            let bytes = state.files.get(&path).ok_or(StatusCode::NoSuchFile)?;
            Ok(Attrs {
                id,
                attrs: FileAttributes {
                    permissions: Some(0o100600),
                    size: Some(bytes.len() as u64),
                    ..FileAttributes::default()
                },
            })
        }

        async fn open(
            &mut self,
            id: u32,
            path: String,
            flags: OpenFlags,
            _: FileAttributes,
        ) -> Result<Handle, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("OPEN {path}"));
            if flags.contains(OpenFlags::CREATE) {
                if state.fault == Fault::OpenDenied {
                    return Err(StatusCode::PermissionDenied);
                }
                if state.files.contains_key(&path) {
                    return Err(StatusCode::Failure);
                }
                state.files.insert(path.clone(), Vec::new());
            } else if !state.files.contains_key(&path) {
                return Err(StatusCode::NoSuchFile);
            }
            Ok(Handle { id, handle: path })
        }

        async fn write(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            data: Vec<u8>,
        ) -> Result<Status, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("WRITE {offset}"));
            state.writes += 1;
            if state.fault == Fault::WriteDenied && state.writes == 2 {
                return Err(StatusCode::PermissionDenied);
            }
            let file = state.files.get_mut(&handle).ok_or(StatusCode::NoSuchFile)?;
            if file.len() != offset as usize {
                return Err(StatusCode::Failure);
            }
            file.extend_from_slice(&data);
            Ok(ok(id))
        }

        async fn read(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            _: u32,
        ) -> Result<Data, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("READ {offset}"));
            state.reads += 1;
            if state.fault == Fault::ReadDenied && state.reads == 2 {
                return Err(StatusCode::PermissionDenied);
            }
            let bytes = state.files.get(&handle).ok_or(StatusCode::NoSuchFile)?;
            let start = offset as usize;
            if start >= bytes.len() {
                return Err(StatusCode::Eof);
            }
            Ok(Data {
                id,
                data: bytes[start..bytes.len().min(start + 6)].to_vec(),
            })
        }

        async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push("CLOSE".into());
            if state.fault == Fault::CloseDenied {
                Err(StatusCode::PermissionDenied)
            } else {
                Ok(ok(id))
            }
        }

        async fn remove(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("REMOVE {path}"));
            if state.fault == Fault::CleanupDenied {
                return Err(StatusCode::PermissionDenied);
            }
            state.files.remove(&path).ok_or(StatusCode::NoSuchFile)?;
            Ok(ok(id))
        }

        async fn extended(
            &mut self,
            id: u32,
            request: String,
            data: Vec<u8>,
        ) -> Result<Packet, Self::Error> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("EXTENDED {request}"));
            if request == "fsync@openssh.com" {
                if state.fault == Fault::FsyncDenied {
                    return Err(StatusCode::PermissionDenied);
                }
                return Ok(Packet::Status(ok(id)));
            }
            if request == "hardlink@openssh.com" || request == "posix-rename@openssh.com" {
                if state.fault == Fault::CommitDenied {
                    return Err(StatusCode::PermissionDenied);
                }
                let mut cursor = data.as_slice();
                let parse = |cursor: &mut &[u8]| -> Option<String> {
                    let len = u32::from_be_bytes(cursor.get(..4)?.try_into().ok()?) as usize;
                    *cursor = cursor.get(4..)?;
                    let value = String::from_utf8(cursor.get(..len)?.to_vec()).ok()?;
                    *cursor = cursor.get(len..)?;
                    Some(value)
                };
                let source = parse(&mut cursor).ok_or(StatusCode::BadMessage)?;
                let destination = parse(&mut cursor).ok_or(StatusCode::BadMessage)?;
                if request == "hardlink@openssh.com" && state.files.contains_key(&destination) {
                    return Err(StatusCode::Failure);
                }
                let content = state
                    .files
                    .get(&source)
                    .ok_or(StatusCode::NoSuchFile)?
                    .clone();
                state.files.insert(destination, content);
                if request == "posix-rename@openssh.com" {
                    state.files.remove(&source);
                }
                return Ok(Packet::Status(ok(id)));
            }
            Err(StatusCode::OpUnsupported)
        }
    }

    async fn matrix_client(fault: Fault) -> (Arc<dyn SftpClient>, Arc<Mutex<MatrixState>>) {
        let state = Arc::new(Mutex::new(MatrixState {
            fault,
            files: HashMap::from([
                ("/fixture/final".into(), b"old-final".to_vec()),
                ("/fixture/source".into(), b"first-second".to_vec()),
                ("/fixture/staging".into(), b"old-staging".to_vec()),
            ]),
            calls: Vec::new(),
            writes: 0,
            reads: 0,
        }));
        let (client_stream, server_stream) = tokio::io::duplex(256 * 1024);
        server::run(server_stream, MatrixFixture(state.clone())).await;
        let client = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            RawSftpClient::connect(client_stream, HostId::new(), HostSessionId::new()),
        )
        .await
        .unwrap()
        .unwrap();
        (client, state)
    }

    #[tokio::test]
    async fn denied_exclusive_open_preserves_preexisting_staging_and_final() {
        let (client, state) = matrix_client(Fault::OpenDenied).await;
        let ownership = StagingOwnership::new();
        let mut reader = std::io::Cursor::new(b"new-payload".to_vec());
        let result = client
            .upload_staged(
                &mut reader,
                "/fixture/staging",
                ownership.clone(),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await;
        assert!(
            matches!(result, Err(StagedUploadFailure::NotCreated(error)) if error.code == ErrorCode::SftpDenied)
        );
        assert!(!ownership.is_owned());
        let state = state.lock().unwrap();
        assert_eq!(state.files["/fixture/staging"], b"old-staging");
        assert_eq!(state.files["/fixture/final"], b"old-final");
        assert_eq!(state.calls, ["OPEN /fixture/staging"]);
    }

    #[tokio::test]
    async fn denied_second_write_keeps_confirmed_progress_below_payload() {
        let (client, state) = matrix_client(Fault::WriteDenied).await;
        let ownership = StagingOwnership::new();
        let mut reader = std::io::Cursor::new(vec![7; CLIENT_CHUNK_BYTES + 17]);
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_clone = progress.clone();
        let result = client
            .upload_staged(
                &mut reader,
                "/fixture/new-staging",
                ownership.clone(),
                CancellationToken::new(),
                Arc::new(move |bytes| progress_clone.lock().unwrap().push(bytes)),
            )
            .await;
        assert!(
            matches!(result, Err(StagedUploadFailure::OwnedFailure(error)) if error.code == ErrorCode::SftpDenied)
        );
        assert!(ownership.is_owned());
        assert_eq!(*progress.lock().unwrap(), vec![CLIENT_CHUNK_BYTES as u64]);
        let state = state.lock().unwrap();
        assert_eq!(
            state.files["/fixture/new-staging"].len(),
            CLIENT_CHUNK_BYTES
        );
        assert_eq!(state.files["/fixture/final"], b"old-final");
        assert_eq!(
            state.calls.iter().filter(|call| *call == "CLOSE").count(),
            1
        );
        assert!(!state.calls.iter().any(|call| call.contains("hardlink")));
    }

    #[tokio::test]
    async fn plan_and_manager_fail_mid_upload_clean_owned_staging_without_commit() {
        use nexus_model::{ConflictPolicy, TransferState};

        let (client, state) = matrix_client(Fault::WriteDenied).await;
        let local = tempfile::tempdir().unwrap();
        let source = local.path().join("final");
        std::fs::write(&source, vec![7; CLIENT_CHUNK_BYTES + 17]).unwrap();
        let selected =
            crate::open_local_source(source.canonicalize().unwrap(), "final".into()).unwrap();
        let store = crate::FilePlanStore::default();
        let plan = store
            .plan_upload(client, vec![selected], "/fixture", ConflictPolicy::Replace)
            .await
            .unwrap();
        let manager = crate::TransferManager::new();
        let jobs = manager
            .execute(
                store
                    .consume(
                        plan.id,
                        plan.host_id,
                        plan.host_session_id,
                        plan.sftp_session_id,
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        let job = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let job = manager
                    .list(None)
                    .into_iter()
                    .find(|job| job.id == jobs[0].id)
                    .unwrap();
                if job.state == TransferState::Failed {
                    break job;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("manager failure watchdog");
        assert_eq!(job.confirmed_bytes, CLIENT_CHUNK_BYTES.to_string());
        assert_eq!(job.error.unwrap().code, ErrorCode::SftpDenied);
        assert!(!job.retryable);
        let state = state.lock().unwrap();
        assert_eq!(state.files["/fixture/final"], b"old-final");
        assert_eq!(state.files["/fixture/staging"], b"old-staging");
        assert_eq!(state.files.len(), 3);
        assert_eq!(
            state
                .calls
                .iter()
                .filter(|call| call.starts_with("REMOVE"))
                .count(),
            1
        );
        assert!(!state.calls.iter().any(|call| call.contains("rename")));
    }

    #[tokio::test]
    async fn denied_second_remote_read_closes_handle_after_nonzero_bytes() {
        let (client, state) = matrix_client(Fault::ReadDenied).await;
        let mut writer = Vec::new();
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_clone = progress.clone();
        let result = client
            .download(
                "/fixture/source",
                &mut writer,
                CancellationToken::new(),
                Arc::new(move |bytes| progress_clone.lock().unwrap().push(bytes)),
            )
            .await;
        assert_eq!(result.unwrap_err().code, ErrorCode::SftpDenied);
        assert_eq!(writer, b"first-");
        assert_eq!(*progress.lock().unwrap(), vec![6]);
        let state = state.lock().unwrap();
        assert_eq!(
            state.calls.iter().filter(|call| *call == "CLOSE").count(),
            1
        );
        assert_eq!(state.files["/fixture/final"], b"old-final");
    }

    #[tokio::test]
    async fn fsync_and_close_denials_do_not_commit_complete_payload() {
        for fault in [Fault::FsyncDenied, Fault::CloseDenied] {
            let (client, state) = matrix_client(fault).await;
            let ownership = StagingOwnership::new();
            let mut reader = std::io::Cursor::new(b"all-bytes".to_vec());
            let progress = Arc::new(Mutex::new(Vec::new()));
            let progress_clone = progress.clone();
            let result = client
                .upload_staged(
                    &mut reader,
                    "/fixture/new-staging",
                    ownership.clone(),
                    CancellationToken::new(),
                    Arc::new(move |bytes| progress_clone.lock().unwrap().push(bytes)),
                )
                .await;
            assert!(
                matches!(result, Err(StagedUploadFailure::OwnedFailure(error)) if error.code == ErrorCode::SftpDenied)
            );
            assert!(ownership.is_owned());
            assert_eq!(*progress.lock().unwrap(), vec![9]);
            let state = state.lock().unwrap();
            assert_eq!(state.files["/fixture/new-staging"], b"all-bytes");
            assert_eq!(state.files["/fixture/final"], b"old-final");
            assert_eq!(
                state.calls.iter().filter(|call| *call == "CLOSE").count(),
                1
            );
            assert!(!state.calls.iter().any(|call| call.contains("hardlink")));
        }
    }

    #[tokio::test]
    async fn known_commit_denial_preserves_final_and_confirmed_commit_cleanup_error_preserves_result()
     {
        let (denied, denied_state) = matrix_client(Fault::CommitDenied).await;
        assert_eq!(
            denied
                .commit_replace("/fixture/staging", "/fixture/final")
                .await
                .unwrap_err()
                .code,
            ErrorCode::SftpDenied
        );
        {
            let denied_state = denied_state.lock().unwrap();
            assert_eq!(denied_state.files["/fixture/final"], b"old-final");
            assert_eq!(denied_state.files["/fixture/staging"], b"old-staging");
        }

        let (confirmed, confirmed_state) = matrix_client(Fault::CleanupDenied).await;
        confirmed_state
            .lock()
            .unwrap()
            .files
            .remove("/fixture/final");
        let error = confirmed
            .commit_new("/fixture/staging", "/fixture/final")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::OutcomeUnknown);
        assert!(error.message.contains("final file exists"));
        let confirmed_state = confirmed_state.lock().unwrap();
        assert_eq!(confirmed_state.files["/fixture/final"], b"old-staging");
        assert_eq!(confirmed_state.files["/fixture/staging"], b"old-staging");
        assert_eq!(
            confirmed_state
                .calls
                .iter()
                .filter(|call| call.starts_with("REMOVE"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn create_new_competitor_between_identity_and_hardlink_is_preserved() {
        let (client, state) = matrix_client(Fault::CommitRace).await;
        let error = client
            .commit_new("/fixture/staging", "/fixture/new-final")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::SftpProtocol);
        let state = state.lock().unwrap();
        assert_eq!(state.files["/fixture/new-final"], b"competitor");
        assert_eq!(state.files["/fixture/staging"], b"old-staging");
        assert_eq!(
            state
                .calls
                .iter()
                .filter(|call| call.starts_with("REMOVE"))
                .count(),
            0
        );
        assert_eq!(
            state
                .calls
                .iter()
                .filter(|call| call.contains("hardlink"))
                .count(),
            1
        );
    }

    #[derive(Clone, Copy)]
    enum LostReply {
        Create,
        Replace,
    }

    async fn recv_packet(stream: &mut tokio::io::DuplexStream) -> Packet {
        use tokio::io::AsyncReadExt;
        let len = stream.read_u32().await.unwrap() as usize;
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload).await.unwrap();
        Packet::try_from(&mut bytes::Bytes::from(payload)).unwrap()
    }

    async fn send_packet(stream: &mut tokio::io::DuplexStream, packet: Packet) {
        use tokio::io::AsyncWriteExt;
        let bytes = bytes::Bytes::try_from(packet).unwrap();
        stream.write_all(&bytes).await.unwrap();
    }

    async fn lost_reply_client(
        mode: LostReply,
    ) -> (
        Arc<dyn SftpClient>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<HashMap<String, Vec<u8>>>>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_clone = calls.clone();
        let files = Arc::new(Mutex::new(HashMap::from([
            ("/fixture/final".into(), b"old-final".to_vec()),
            ("/fixture/staging".into(), b"new-final".to_vec()),
        ])));
        let files_clone = files.clone();
        let (client_stream, mut server_stream) = tokio::io::duplex(128 * 1024);
        tokio::spawn(async move {
            assert!(matches!(
                recv_packet(&mut server_stream).await,
                Packet::Init(_)
            ));
            let mut version = Version::new();
            version
                .extensions
                .insert("posix-rename@openssh.com".into(), "1".into());
            send_packet(&mut server_stream, Packet::Version(version)).await;
            let realpath = recv_packet(&mut server_stream).await;
            assert!(matches!(realpath, Packet::RealPath(_)));
            send_packet(
                &mut server_stream,
                Packet::Name(Name {
                    id: realpath.get_request_id(),
                    files: vec![File::dummy("/fixture")],
                }),
            )
            .await;
            let mutation = recv_packet(&mut server_stream).await;
            match (mode, mutation) {
                (LostReply::Create, Packet::Open(open)) => {
                    assert!(open.pflags.contains(OpenFlags::CREATE | OpenFlags::EXCLUDE));
                    files_clone
                        .lock()
                        .unwrap()
                        .insert(open.filename, Vec::new());
                    calls_clone
                        .lock()
                        .unwrap()
                        .push("OPEN executed; reply lost".into());
                }
                (LostReply::Replace, Packet::Extended(extended)) => {
                    assert_eq!(extended.request, "posix-rename@openssh.com");
                    let mut files = files_clone.lock().unwrap();
                    let payload = files.remove("/fixture/staging").unwrap();
                    files.insert("/fixture/final".into(), payload);
                    calls_clone
                        .lock()
                        .unwrap()
                        .push("REPLACE executed; reply lost".into());
                }
                _ => panic!("unexpected protocol request"),
            }
            // Dropping the stream after executing the mutation is a real lost reply.
        });
        let client = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            RawSftpClient::connect(client_stream, HostId::new(), HostSessionId::new()),
        )
        .await
        .unwrap()
        .unwrap();
        (client, calls, files)
    }

    #[tokio::test]
    async fn lost_exclusive_create_reply_never_establishes_ownership_or_removes_staging() {
        let (client, calls, files) = lost_reply_client(LostReply::Create).await;
        let ownership = StagingOwnership::new();
        let mut reader = std::io::Cursor::new(b"new-payload".to_vec());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.upload_staged(
                &mut reader,
                "/fixture/staging",
                ownership.clone(),
                CancellationToken::new(),
                Arc::new(|_| {}),
            ),
        )
        .await
        .unwrap();
        assert!(
            matches!(result, Err(StagedUploadFailure::CreationOutcomeUnknown(error)) if error.code == ErrorCode::OutcomeUnknown)
        );
        assert!(!ownership.is_owned());
        assert_eq!(*calls.lock().unwrap(), ["OPEN executed; reply lost"]);
        assert_eq!(files.lock().unwrap()["/fixture/final"], b"old-final");
        assert!(files.lock().unwrap().contains_key("/fixture/staging"));
    }

    #[tokio::test]
    async fn executed_replace_with_lost_reply_is_outcome_unknown_without_replay() {
        let (client, calls, files) = lost_reply_client(LostReply::Replace).await;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.commit_replace("/fixture/staging", "/fixture/final"),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, ErrorCode::OutcomeUnknown);
        assert_eq!(*calls.lock().unwrap(), ["REPLACE executed; reply lost"]);
        assert_eq!(files.lock().unwrap()["/fixture/final"], b"new-final");
        assert!(!files.lock().unwrap().contains_key("/fixture/staging"));
    }
}
