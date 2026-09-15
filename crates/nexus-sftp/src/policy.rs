use crate::{
    EntryIdentity, SftpClient, display_name, join_remote, validate_child_name,
    validate_remote_path, validate_windows_file_name,
};
use nexus_model::{
    AppError, ConflictPolicy, ErrorCode, FileOperationKind, FileOperationPlan, FilePlanId,
    FilePlanItem, HostId, HostSessionId, OperationRisk, RemoteEntryKind, SftpSessionId,
};
use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

const PLAN_LIFETIME: Duration = Duration::from_secs(5 * 60);
const PLAN_CAP: usize = 100;

#[derive(Clone, Debug)]
pub struct LocalItem {
    pub path: PathBuf,
    pub display_name: String,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
    pub identity: LocalIdentity,
    pub handle: Arc<File>,
}

#[derive(Clone, Debug)]
pub struct LocalDirectory {
    pub path: PathBuf,
    pub display_name: String,
    pub identity: LocalIdentity,
    pub handle: Arc<File>,
}

#[derive(Clone, Debug)]
pub struct RemoteSource {
    pub path: String,
    pub name: String,
    pub size: Option<u64>,
    pub identity: EntryIdentity,
}

#[derive(Clone, Debug)]
pub struct PlannedUpload {
    pub source: LocalItem,
    pub destination: String,
    pub action: DestinationAction<EntryIdentity>,
}

#[derive(Clone, Debug)]
pub struct PlannedDownload {
    pub source: RemoteSource,
    pub destination: LocalDirectory,
    pub file_name: String,
    pub action: DestinationAction<LocalIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalIdentity {
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
    pub object_id: LocalObjectId,
}

/// Immutable filesystem object identity. Mutable size and timestamps remain
/// separate preconditions in [`LocalIdentity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalObjectId {
    pub storage_id: u64,
    pub file_id: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DestinationAction<T> {
    Skip,
    CreateNew,
    ReplaceExisting(T),
}

#[derive(Clone, Debug)]
pub enum MutationSpec {
    CreateDirectory {
        path: String,
    },
    Rename {
        source: String,
        destination: String,
        expected_source: EntryIdentity,
    },
    Delete {
        path: String,
        kind: RemoteEntryKind,
        expected: EntryIdentity,
    },
}

#[derive(Clone)]
pub enum PlanPayload {
    Upload {
        client: std::sync::Arc<dyn SftpClient>,
        policy: ConflictPolicy,
        items: Vec<PlannedUpload>,
    },
    Download {
        client: std::sync::Arc<dyn SftpClient>,
        policy: ConflictPolicy,
        items: Vec<PlannedDownload>,
    },
    Mutation {
        client: std::sync::Arc<dyn SftpClient>,
        spec: MutationSpec,
    },
}

pub struct InternalPlan {
    pub view: FileOperationPlan,
    pub payload: PlanPayload,
}

struct StoredPlan {
    plan: Option<InternalPlan>,
    expires: Instant,
}

pub struct FilePlanStore {
    plans: Arc<Mutex<HashMap<FilePlanId, StoredPlan>>>,
    lifetime: Duration,
}

impl Default for FilePlanStore {
    fn default() -> Self {
        Self {
            plans: Arc::default(),
            lifetime: PLAN_LIFETIME,
        }
    }
}

impl FilePlanStore {
    pub fn revoke_session(&self, host: HostId, host_session: HostSessionId) {
        if let Ok(mut plans) = self.plans.lock() {
            plans.retain(|_, stored| {
                stored.plan.as_ref().is_some_and(|plan| {
                    plan.view.host_id != host || plan.view.host_session_id != host_session
                })
            });
        }
    }

    /// Discards only the named pending plan. If execution consumed it first,
    /// the active job owns its resources and this operation is idempotent.
    pub fn discard(
        &self,
        id: FilePlanId,
        host: HostId,
        host_session: HostSessionId,
        sftp: SftpSessionId,
    ) -> Result<(), AppError> {
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| policy_error("The file plan store is unavailable."))?;
        let Some(stored) = plans.get(&id) else {
            return Ok(());
        };
        let Some(plan) = stored.plan.as_ref() else {
            plans.remove(&id);
            return Ok(());
        };
        if (
            plan.view.host_id,
            plan.view.host_session_id,
            plan.view.sftp_session_id,
        ) != (host, host_session, sftp)
        {
            return Err(policy_error(
                "The file-operation plan does not belong to this host session.",
            ));
        }
        plans.remove(&id);
        Ok(())
    }

    pub async fn plan_upload(
        &self,
        client: std::sync::Arc<dyn SftpClient>,
        sources: Vec<LocalItem>,
        remote_directory: &str,
        policy: ConflictPolicy,
    ) -> Result<FileOperationPlan, AppError> {
        validate_remote_path(remote_directory)?;
        if sources.is_empty() {
            return Err(policy_error("Select at least one regular file."));
        }
        let mut planned = Vec::with_capacity(sources.len());
        let mut items = Vec::with_capacity(sources.len());
        for source in sources {
            validate_local_source(&source)?;
            validate_child_name(&source.display_name)?;
            let base = join_remote(remote_directory, &source.display_name)?;
            let existing = client.identity(&base).await?;
            let (destination, action) = match (existing, policy) {
                (None, _) if supports(&client.info(), "hardlink@openssh.com", "1") => {
                    (base, DestinationAction::CreateNew)
                }
                (None, _) => {
                    return Err(policy_error(
                        "Safe new-file finalization requires hardlink@openssh.com v1.",
                    ));
                }
                (Some(_), ConflictPolicy::Skip) => (base, DestinationAction::Skip),
                (Some(identity), ConflictPolicy::Replace)
                    if identity.kind == RemoteEntryKind::File
                        && supports(&client.info(), "posix-rename@openssh.com", "1") =>
                {
                    (base, DestinationAction::ReplaceExisting(identity))
                }
                (Some(_), ConflictPolicy::Replace) => {
                    return Err(policy_error(
                        "Replace is limited to an existing regular file.",
                    ));
                }
                (Some(_), ConflictPolicy::KeepBoth) => {
                    if !supports(&client.info(), "hardlink@openssh.com", "1") {
                        return Err(policy_error(
                            "Safe Keep both finalization requires hardlink@openssh.com v1.",
                        ));
                    }
                    let candidate =
                        keep_both_remote(client.as_ref(), remote_directory, &source.display_name)
                            .await?;
                    (candidate, DestinationAction::CreateNew)
                }
            };
            items.push(FilePlanItem {
                source_display: display_name(&source.display_name),
                destination_display: display_name(&destination),
                size_bytes: Some(source.size.to_string()),
            });
            planned.push(PlannedUpload {
                source,
                destination,
                action,
            });
        }
        self.insert(
            client.info(),
            FileOperationKind::Upload,
            OperationRisk::Moderate,
            Some(policy),
            items,
            PlanPayload::Upload {
                client,
                policy,
                items: planned,
            },
        )
    }

    pub async fn plan_download(
        &self,
        client: std::sync::Arc<dyn SftpClient>,
        remote_paths: Vec<String>,
        local_directory: LocalDirectory,
        policy: ConflictPolicy,
    ) -> Result<FileOperationPlan, AppError> {
        if remote_paths.is_empty() {
            return Err(policy_error("Select at least one regular file."));
        }
        let mut planned = Vec::with_capacity(remote_paths.len());
        let mut items = Vec::with_capacity(remote_paths.len());
        for path in remote_paths {
            let entry = client.stat(&path).await?;
            if entry.kind != RemoteEntryKind::File {
                return Err(policy_error("Only regular files can be downloaded."));
            }
            validate_windows_file_name(&entry.name)?;
            validate_local_directory_handle(&local_directory)?;
            let mut destination = local_directory.path.join(&entry.name);
            let existing = local_identity(&destination)?;
            let action = match (existing, policy) {
                (None, _) => DestinationAction::CreateNew,
                (Some(_), ConflictPolicy::Skip) => DestinationAction::Skip,
                (Some(value), ConflictPolicy::Replace) => DestinationAction::ReplaceExisting(value),
                (Some(_), ConflictPolicy::KeepBoth) => {
                    destination = keep_both_local(&local_directory.path, &entry.name)?;
                    DestinationAction::CreateNew
                }
            };
            let identity = client.identity(&path).await?.ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "The remote source no longer exists.")
            })?;
            items.push(FilePlanItem {
                source_display: entry.display_name,
                destination_display: display_name(&destination.to_string_lossy()),
                size_bytes: entry.size_bytes,
            });
            planned.push(PlannedDownload {
                source: RemoteSource {
                    path,
                    name: entry.name,
                    size: identity.size,
                    identity,
                },
                destination: local_directory.clone(),
                file_name: destination
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| policy_error("The local filename is unavailable."))?
                    .to_owned(),
                action,
            });
        }
        self.insert(
            client.info(),
            FileOperationKind::Download,
            OperationRisk::Moderate,
            Some(policy),
            items,
            PlanPayload::Download {
                client,
                policy,
                items: planned,
            },
        )
    }

    pub fn plan_mutation(
        &self,
        client: std::sync::Arc<dyn SftpClient>,
        spec: MutationSpec,
    ) -> Result<FileOperationPlan, AppError> {
        let (kind, risk, item) = match &spec {
            MutationSpec::CreateDirectory { path } => {
                validate_remote_path(path)?;
                (
                    FileOperationKind::CreateDirectory,
                    OperationRisk::Low,
                    FilePlanItem {
                        source_display: "New directory".into(),
                        destination_display: display_name(path),
                        size_bytes: None,
                    },
                )
            }
            MutationSpec::Rename {
                source,
                destination,
                ..
            } => {
                validate_remote_path(source)?;
                validate_remote_path(destination)?;
                (
                    FileOperationKind::Rename,
                    OperationRisk::Moderate,
                    FilePlanItem {
                        source_display: display_name(source),
                        destination_display: display_name(destination),
                        size_bytes: None,
                    },
                )
            }
            MutationSpec::Delete { path, .. } => {
                validate_remote_path(path)?;
                (
                    FileOperationKind::Delete,
                    OperationRisk::Destructive,
                    FilePlanItem {
                        source_display: display_name(path),
                        destination_display: "Permanent deletion".into(),
                        size_bytes: None,
                    },
                )
            }
        };
        self.insert(
            client.info(),
            kind,
            risk,
            None,
            vec![item],
            PlanPayload::Mutation { client, spec },
        )
    }

    pub fn consume(
        &self,
        id: FilePlanId,
        host: HostId,
        host_session: HostSessionId,
        sftp: SftpSessionId,
    ) -> Result<InternalPlan, AppError> {
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| policy_error("The file plan store is unavailable."))?;
        let expired = plans
            .get(&id)
            .is_some_and(|stored| Instant::now() > stored.expires);
        if expired {
            plans.remove(&id);
            return Err(policy_error("The file-operation plan expired."));
        }
        let stored = plans.get_mut(&id).ok_or_else(|| {
            policy_error("The file-operation plan is missing or was already used.")
        })?;
        let plan = stored
            .plan
            .take()
            .ok_or_else(|| policy_error("The file-operation plan was already used."))?;
        if plan.view.host_id != host
            || plan.view.host_session_id != host_session
            || plan.view.sftp_session_id != sftp
        {
            return Err(policy_error(
                "The file-operation plan does not belong to this host session.",
            ));
        }
        Ok(plan)
    }

    fn insert(
        &self,
        info: nexus_model::SftpSessionInfo,
        kind: FileOperationKind,
        risk: OperationRisk,
        conflict_policy: Option<ConflictPolicy>,
        items: Vec<FilePlanItem>,
        payload: PlanPayload,
    ) -> Result<FileOperationPlan, AppError> {
        let id = FilePlanId::new();
        let expires = Instant::now() + self.lifetime;
        let view = FileOperationPlan {
            id,
            host_id: info.host_id,
            host_session_id: info.host_session_id,
            sftp_session_id: info.id,
            kind,
            risk,
            conflict_policy,
            items,
            expires_at: (chrono::Utc::now()
                + chrono::Duration::from_std(self.lifetime)
                    .map_err(|_| policy_error("The file plan lifetime is invalid."))?)
            .to_rfc3339(),
        };
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| policy_error("The file plan store is unavailable."))?;
        plans.retain(|_, value| value.expires > Instant::now() && value.plan.is_some());
        if plans.len() >= PLAN_CAP {
            return Err(policy_error(
                "Too many file-operation plans are awaiting approval.",
            ));
        }
        plans.insert(
            id,
            StoredPlan {
                plan: Some(InternalPlan {
                    view: view.clone(),
                    payload,
                }),
                expires,
            },
        );
        schedule_plan_expiry(Arc::downgrade(&self.plans), id, expires);
        Ok(view)
    }
}

fn schedule_plan_expiry(
    plans: Weak<Mutex<HashMap<FilePlanId, StoredPlan>>>,
    id: FilePlanId,
    expires: Instant,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(expires)).await;
        let Some(plans) = plans.upgrade() else {
            return;
        };
        if let Ok(mut plans) = plans.lock()
            && plans
                .get(&id)
                .is_some_and(|stored| stored.expires == expires)
        {
            plans.remove(&id);
        }
    });
}

fn supports(info: &nexus_model::SftpSessionInfo, name: &str, version: &str) -> bool {
    info.extensions
        .iter()
        .any(|extension| extension.name == name && extension.version == version)
}

async fn keep_both_remote(
    client: &dyn SftpClient,
    directory: &str,
    name: &str,
) -> Result<String, AppError> {
    let (stem, extension) = split_name(name);
    for number in 1..=1_000 {
        let candidate = join_remote(directory, &format!("{stem} ({number}){extension}"))?;
        if client.identity(&candidate).await?.is_none() {
            return Ok(candidate);
        }
    }
    Err(policy_error("No available Keep both name was found."))
}

fn keep_both_local(directory: &std::path::Path, name: &str) -> Result<PathBuf, AppError> {
    let (stem, extension) = split_name(name);
    for number in 1..=1_000 {
        let candidate = directory.join(format!("{stem} ({number}){extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(policy_error("No available Keep both name was found."))
}

fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

pub fn local_identity(path: &std::path::Path) -> Result<Option<LocalIdentity>, AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || is_reparse(&metadata) {
                return Err(policy_error(
                    "The local destination is not a regular non-reparse file.",
                ));
            }
            let handle = open_file_identity_handle(path)?;
            let handle_metadata = handle.metadata().map_err(|_| {
                local_access_error("The local destination identity could not be read.")
            })?;
            if !handle_metadata.file_type().is_file() || is_reparse(&handle_metadata) {
                return Err(policy_error(
                    "The local destination is not a regular non-reparse file.",
                ));
            }
            Ok(Some(handle_identity(&handle, &handle_metadata)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(AppError::new(
            ErrorCode::LocalAccess,
            "The local destination metadata could not be read.",
        )),
    }
}

pub fn validate_local_directory(path: &std::path::Path) -> Result<(), AppError> {
    for component in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if component.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(component).map_err(|_| {
            AppError::new(
                ErrorCode::LocalAccess,
                "The selected local directory ancestry is unavailable.",
            )
        })?;
        if is_reparse(&metadata) {
            return Err(policy_error(
                "The selected local directory ancestry contains a symlink or reparse point.",
            ));
        }
    }
    if !std::fs::symlink_metadata(path)
        .map_err(|_| local_access_error("The selected local directory is unavailable."))?
        .is_dir()
    {
        return Err(policy_error(
            "The selected local destination is not a directory.",
        ));
    }
    Ok(())
}

pub fn open_local_source(path: PathBuf, display_name: String) -> Result<LocalItem, AppError> {
    if let Some(parent) = path.parent() {
        validate_local_directory(parent)?;
    }
    let path_metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| local_access_error("A selected local file is unavailable."))?;
    if !path_metadata.file_type().is_file() || is_reparse(&path_metadata) {
        return Err(local_access_error(
            "Uploads accept only regular non-reparse files.",
        ));
    }
    let handle = open_source_handle(&path)?;
    let handle_metadata = handle
        .metadata()
        .map_err(|_| local_access_error("A selected local file identity is unavailable."))?;
    if !handle_metadata.file_type().is_file() || is_reparse(&handle_metadata) {
        return Err(local_access_error(
            "Uploads accept only regular non-reparse files.",
        ));
    }
    let identity = handle_identity(&handle, &handle_metadata)?;
    let path_handle = open_file_identity_handle(&path)?;
    let path_handle_metadata = path_handle
        .metadata()
        .map_err(|_| local_access_error("A selected local file identity is unavailable."))?;
    if identity != handle_identity(&path_handle, &path_handle_metadata)? {
        return Err(local_access_error(
            "The selected local file changed while it was opened.",
        ));
    }
    Ok(LocalItem {
        path,
        display_name,
        size: identity.size,
        modified: identity.modified,
        identity,
        handle: Arc::new(handle),
    })
}

pub fn open_local_directory(path: PathBuf) -> Result<LocalDirectory, AppError> {
    validate_local_directory(&path)?;
    let handle = open_directory_handle(&path)?;
    let handle_metadata = handle
        .metadata()
        .map_err(|_| local_access_error("The selected directory identity is unavailable."))?;
    let identity = handle_identity(&handle, &handle_metadata)?;
    let path_handle = open_directory_handle(&path)?;
    let path_metadata = path_handle
        .metadata()
        .map_err(|_| local_access_error("The selected directory identity is unavailable."))?;
    let path_identity = handle_identity(&path_handle, &path_metadata)?;
    if identity != path_identity {
        return Err(local_access_error(
            "The selected directory changed while it was opened.",
        ));
    }
    let display_name = display_name(&path.to_string_lossy());
    Ok(LocalDirectory {
        path,
        display_name,
        identity,
        handle: Arc::new(handle),
    })
}

pub fn validate_local_source(item: &LocalItem) -> Result<(), AppError> {
    if let Some(parent) = item.path.parent() {
        validate_local_directory(parent)?;
    }
    let path_metadata = std::fs::symlink_metadata(&item.path).map_err(|_| {
        AppError::new(
            ErrorCode::LocalAccess,
            "The selected local source is unavailable.",
        )
    })?;
    let path_handle = open_file_identity_handle(&item.path)?;
    let path_handle_metadata = path_handle
        .metadata()
        .map_err(|_| local_access_error("The selected local source is unavailable."))?;
    let handle_metadata = item
        .handle
        .metadata()
        .map_err(|_| local_access_error("The selected local source handle is unavailable."))?;
    if !path_metadata.file_type().is_file()
        || is_reparse(&path_metadata)
        || !path_handle_metadata.file_type().is_file()
        || is_reparse(&path_handle_metadata)
        || handle_identity(&path_handle, &path_handle_metadata)? != item.identity
        || handle_identity(&item.handle, &handle_metadata)? != item.identity
    {
        return Err(policy_error(
            "The selected local source changed after approval.",
        ));
    }
    Ok(())
}

pub fn validate_local_directory_handle(directory: &LocalDirectory) -> Result<(), AppError> {
    validate_local_directory(&directory.path)?;
    let path_handle = open_directory_handle(&directory.path)?;
    let path_metadata = path_handle
        .metadata()
        .map_err(|_| local_access_error("The selected local directory is unavailable."))?;
    let path_identity = handle_identity(&path_handle, &path_metadata)?;
    let selected_metadata = directory
        .handle
        .metadata()
        .map_err(|_| local_access_error("The selected directory handle is unavailable."))?;
    let selected_identity = handle_identity(&directory.handle, &selected_metadata)?;
    if !same_local_object(&path_identity, &directory.identity)
        || !same_local_object(&selected_identity, &directory.identity)
    {
        return Err(policy_error(
            "The selected local directory changed after selection.",
        ));
    }
    Ok(())
}

fn same_local_object(left: &LocalIdentity, right: &LocalIdentity) -> bool {
    left.object_id == right.object_id
}

fn handle_identity(handle: &File, metadata: &std::fs::Metadata) -> Result<LocalIdentity, AppError> {
    let id = fs_id::FileID::new(handle).map_err(|_| {
        local_access_error("Strong local filesystem identity is unavailable for this object.")
    })?;
    Ok(LocalIdentity {
        size: metadata.len(),
        modified: metadata.modified().ok(),
        object_id: LocalObjectId {
            storage_id: id.storage_id(),
            file_id: id.internal_file_id(),
        },
    })
}

#[cfg(windows)]
fn open_directory_handle(path: &std::path::Path) -> Result<File, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001 | 0x0000_0002)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)
        .map_err(|_| local_access_error("The selected directory could not be opened safely."))
}
#[cfg(not(windows))]
fn open_directory_handle(path: &std::path::Path) -> Result<File, AppError> {
    File::open(path)
        .map_err(|_| local_access_error("The selected directory could not be opened safely."))
}

#[cfg(windows)]
fn open_source_handle(path: &std::path::Path) -> Result<File, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| local_access_error("The selected local file could not be locked for reading."))
}

#[cfg(windows)]
fn open_file_identity_handle(path: &std::path::Path) -> Result<File, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001)
        .custom_flags(0x0020_0000)
        .open(path)
        .map_err(|_| local_access_error("The local filesystem object could not be identified."))
}
#[cfg(not(windows))]
fn open_file_identity_handle(path: &std::path::Path) -> Result<File, AppError> {
    File::open(path)
        .map_err(|_| local_access_error("The local filesystem object could not be identified."))
}
#[cfg(not(windows))]
fn open_source_handle(path: &std::path::Path) -> Result<File, AppError> {
    File::open(path).map_err(|_| local_access_error("The selected local file could not be opened."))
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn policy_error(message: &'static str) -> AppError {
    AppError::new(ErrorCode::FilePolicy, message)
}

fn local_access_error(message: &'static str) -> AppError {
    AppError::new(ErrorCode::LocalAccess, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nexus_model::{
        DirectoryListing, HostSessionId, RemoteEntry, SftpLimits, SftpSessionId, SftpSessionInfo,
    };
    use std::sync::Arc;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio_util::sync::CancellationToken;

    struct PlanClient {
        info: SftpSessionInfo,
    }

    impl PlanClient {
        fn new() -> Arc<Self> {
            Self::with_extensions(Vec::new())
        }
        fn with_extensions(extensions: Vec<nexus_model::SftpExtension>) -> Arc<Self> {
            Arc::new(Self {
                info: SftpSessionInfo {
                    id: SftpSessionId::new(),
                    host_id: HostId::new(),
                    host_session_id: HostSessionId::new(),
                    protocol_version: 3,
                    extensions,
                    limits: SftpLimits {
                        max_packet_bytes: None,
                        max_read_bytes: None,
                        max_write_bytes: None,
                        max_open_handles: None,
                        client_chunk_bytes: "65536".into(),
                        listing_entry_cap: 5_000,
                    },
                    root_path: "/home/test".into(),
                },
            })
        }
    }

    fn unused() -> AppError {
        AppError::new(ErrorCode::SftpProtocol, "Unused mock operation.")
    }

    #[async_trait]
    impl SftpClient for PlanClient {
        fn info(&self) -> SftpSessionInfo {
            self.info.clone()
        }
        async fn list(&self, _: &str, _: CancellationToken) -> Result<DirectoryListing, AppError> {
            Err(unused())
        }
        async fn stat(&self, _: &str) -> Result<RemoteEntry, AppError> {
            Ok(RemoteEntry {
                name: "remote-source.bin".into(),
                display_name: "remote-source.bin".into(),
                path: "/home/test/remote-source.bin".into(),
                kind: RemoteEntryKind::File,
                size_bytes: Some("4".into()),
                modified_at: None,
                permissions: None,
                uid: None,
                gid: None,
            })
        }
        async fn identity(&self, path: &str) -> Result<Option<EntryIdentity>, AppError> {
            if path.ends_with("remote-source.bin") {
                Ok(Some(EntryIdentity {
                    kind: RemoteEntryKind::File,
                    size: Some(4),
                    modified: Some(1),
                }))
            } else {
                Ok(None)
            }
        }
        async fn create_dir(&self, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn remove_file(&self, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn remove_dir(&self, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn rename_noclobber(&self, _: &str, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn upload_staged(
            &self,
            _: &mut (dyn AsyncRead + Unpin + Send),
            _: &str,
            _: crate::StagingOwnership,
            _: CancellationToken,
            _: crate::Progress,
        ) -> crate::StagedUploadResult {
            Err(crate::StagedUploadFailure::NotCreated(unused()))
        }
        async fn download(
            &self,
            _: &str,
            _: &mut (dyn AsyncWrite + Unpin + Send),
            _: CancellationToken,
            _: crate::Progress,
        ) -> Result<u64, AppError> {
            Err(unused())
        }
        async fn commit_new(&self, _: &str, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn commit_replace(&self, _: &str, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn remove_owned_staging(&self, _: &str) {}
        async fn close(&self) {}
    }

    #[test]
    fn keep_both_names_are_bounded_and_preserve_extension() {
        assert_eq!(split_name("archive.tar.gz"), ("archive.tar", ".gz"));
        assert_eq!(split_name("README"), ("README", ""));
    }

    #[test]
    fn plans_are_owned_one_shot_and_expire() {
        let client = PlanClient::new();
        let store = FilePlanStore::default();
        let first = store
            .plan_mutation(
                client.clone(),
                MutationSpec::CreateDirectory {
                    path: "/home/test/new".into(),
                },
            )
            .unwrap();
        assert_eq!(
            store
                .consume(
                    first.id,
                    HostId::new(),
                    first.host_session_id,
                    first.sftp_session_id
                )
                .err()
                .unwrap()
                .code,
            ErrorCode::FilePolicy
        );
        assert_eq!(
            store
                .consume(
                    first.id,
                    first.host_id,
                    first.host_session_id,
                    first.sftp_session_id
                )
                .err()
                .unwrap()
                .code,
            ErrorCode::FilePolicy
        );

        let replay = store
            .plan_mutation(
                client.clone(),
                MutationSpec::CreateDirectory {
                    path: "/home/test/once".into(),
                },
            )
            .unwrap();
        store
            .consume(
                replay.id,
                replay.host_id,
                replay.host_session_id,
                replay.sftp_session_id,
            )
            .unwrap();
        assert_eq!(
            store
                .consume(
                    replay.id,
                    replay.host_id,
                    replay.host_session_id,
                    replay.sftp_session_id
                )
                .err()
                .unwrap()
                .code,
            ErrorCode::FilePolicy
        );

        let expired = store
            .plan_mutation(
                client,
                MutationSpec::CreateDirectory {
                    path: "/home/test/expired".into(),
                },
            )
            .unwrap();
        store
            .plans
            .lock()
            .unwrap()
            .get_mut(&expired.id)
            .unwrap()
            .expires = Instant::now() - Duration::from_millis(1);
        assert_eq!(
            store
                .consume(
                    expired.id,
                    expired.host_id,
                    expired.host_session_id,
                    expired.sftp_session_id
                )
                .err()
                .unwrap()
                .code,
            ErrorCode::FilePolicy
        );
    }

    #[tokio::test]
    async fn upload_plan_rejects_servers_without_safe_no_clobber_extension() {
        let client = PlanClient::new();
        let store = FilePlanStore::default();
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("data.bin");
        std::fs::write(&source_path, [1_u8]).unwrap();
        let error = store
            .plan_upload(
                client,
                vec![
                    open_local_source(source_path.canonicalize().unwrap(), "data.bin".into())
                        .unwrap(),
                ],
                "/home/test",
                ConflictPolicy::Skip,
            )
            .await
            .expect_err("unsafe server must be rejected before approval");
        assert_eq!(error.code, ErrorCode::FilePolicy);
    }

    #[cfg(windows)]
    #[test]
    fn selected_directory_handle_blocks_parent_replacement() {
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("destination");
        std::fs::create_dir(&destination).unwrap();
        let selected = open_local_directory(destination.canonicalize().unwrap()).unwrap();
        assert!(std::fs::rename(&destination, parent.path().join("moved")).is_err());
        validate_local_directory_handle(&selected).unwrap();
    }

    #[tokio::test]
    async fn replace_preference_plans_absent_targets_as_create_new() {
        let client = PlanClient::with_extensions(vec![
            nexus_model::SftpExtension {
                name: "hardlink@openssh.com".into(),
                version: "1".into(),
            },
            nexus_model::SftpExtension {
                name: "posix-rename@openssh.com".into(),
                version: "1".into(),
            },
        ]);
        let store = FilePlanStore::default();
        let local = tempfile::tempdir().unwrap();
        let upload_path = local.path().join("upload.bin");
        std::fs::write(&upload_path, b"data").unwrap();
        let upload = store
            .plan_upload(
                client.clone(),
                vec![
                    open_local_source(upload_path.canonicalize().unwrap(), "upload.bin".into())
                        .unwrap(),
                ],
                "/home/test",
                ConflictPolicy::Replace,
            )
            .await
            .unwrap();
        let upload = store
            .consume(
                upload.id,
                upload.host_id,
                upload.host_session_id,
                upload.sftp_session_id,
            )
            .unwrap();
        assert!(matches!(
            upload.payload,
            PlanPayload::Upload { ref items, .. }
                if items.len() == 1 && items[0].action == DestinationAction::CreateNew
        ));

        let destination = open_local_directory(local.path().canonicalize().unwrap()).unwrap();
        let download = store
            .plan_download(
                client,
                vec!["/home/test/remote-source.bin".into()],
                destination,
                ConflictPolicy::Replace,
            )
            .await
            .unwrap();
        let download = store
            .consume(
                download.id,
                download.host_id,
                download.host_session_id,
                download.sftp_session_id,
            )
            .unwrap();
        assert!(matches!(
            download.payload,
            PlanPayload::Download { ref items, .. }
                if items.len() == 1 && items[0].action == DestinationAction::CreateNew
        ));
    }

    #[tokio::test]
    async fn download_approval_displays_full_final_destination() {
        let client = PlanClient::new();
        let store = FilePlanStore::default();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();

        let first_plan = store
            .plan_download(
                client.clone(),
                vec!["/home/test/remote-source.bin".into()],
                open_local_directory(first.path().canonicalize().unwrap()).unwrap(),
                ConflictPolicy::Skip,
            )
            .await
            .unwrap();
        let second_plan = store
            .plan_download(
                client.clone(),
                vec!["/home/test/remote-source.bin".into()],
                open_local_directory(second.path().canonicalize().unwrap()).unwrap(),
                ConflictPolicy::Skip,
            )
            .await
            .unwrap();
        let first_display = &first_plan.items[0].destination_display;
        let second_display = &second_plan.items[0].destination_display;
        assert_ne!(first_display, second_display);
        assert!(first_display.ends_with("remote-source.bin"));
        assert!(second_display.ends_with("remote-source.bin"));

        std::fs::write(first.path().join("remote-source.bin"), b"existing").unwrap();
        let keep_both = store
            .plan_download(
                client,
                vec!["/home/test/remote-source.bin".into()],
                open_local_directory(first.path().canonicalize().unwrap()).unwrap(),
                ConflictPolicy::KeepBoth,
            )
            .await
            .unwrap();
        assert!(
            keep_both.items[0]
                .destination_display
                .ends_with("remote-source (1).bin")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_object_identity_uses_volume_and_full_file_id() {
        use std::os::windows::fs::{FileTimesExt, MetadataExt};

        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first.bin");
        let second_path = directory.path().join("second.bin");
        std::fs::write(&first_path, b"same-size").unwrap();
        std::fs::write(&second_path, b"same-size").unwrap();
        let timestamp = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let times = std::fs::FileTimes::new()
            .set_created(timestamp)
            .set_modified(timestamp);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&first_path)
            .unwrap()
            .set_times(times)
            .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&second_path)
            .unwrap()
            .set_times(times)
            .unwrap();
        let first_metadata = std::fs::metadata(&first_path).unwrap();
        let second_metadata = std::fs::metadata(&second_path).unwrap();
        assert_eq!(
            first_metadata.creation_time(),
            second_metadata.creation_time()
        );
        assert_eq!(first_metadata.file_size(), second_metadata.file_size());

        let first = local_identity(&first_path).unwrap().unwrap();
        let second = local_identity(&second_path).unwrap().unwrap();
        assert_eq!(first.size, second.size);
        assert_ne!(first.object_id, second.object_id);

        let first_again = local_identity(&first_path).unwrap().unwrap();
        assert_eq!(first.object_id, first_again.object_id);

        std::fs::write(&first_path, b"changed-size-and-content").unwrap();
        let changed = local_identity(&first_path).unwrap().unwrap();
        assert_eq!(first.object_id, changed.object_id);
        assert_ne!(first, changed);

        let first_directory =
            open_local_directory(directory.path().canonicalize().unwrap()).unwrap();
        let other = tempfile::tempdir().unwrap();
        let second_directory = open_local_directory(other.path().canonicalize().unwrap()).unwrap();
        assert_ne!(
            first_directory.identity.object_id,
            second_directory.identity.object_id
        );
    }

    #[cfg(windows)]
    #[tokio::test(start_paused = true)]
    async fn pending_plan_discard_and_idle_expiry_release_directory_handles() {
        let client = PlanClient::new();
        let parent = tempfile::tempdir().unwrap();

        let discarded_path = parent.path().join("discarded");
        std::fs::create_dir(&discarded_path).unwrap();
        let discarded_store = FilePlanStore::default();
        let discarded = discarded_store
            .plan_download(
                client.clone(),
                vec!["/home/test/remote-source.bin".into()],
                open_local_directory(discarded_path.canonicalize().unwrap()).unwrap(),
                ConflictPolicy::Skip,
            )
            .await
            .unwrap();
        assert!(std::fs::rename(&discarded_path, parent.path().join("still-locked")).is_err());
        discarded_store
            .discard(
                discarded.id,
                discarded.host_id,
                discarded.host_session_id,
                discarded.sftp_session_id,
            )
            .unwrap();
        std::fs::rename(&discarded_path, parent.path().join("discard-released")).unwrap();

        let consumed_path = parent.path().join("consumed");
        std::fs::create_dir(&consumed_path).unwrap();
        let consumed_store = FilePlanStore::default();
        let consumed_view = consumed_store
            .plan_download(
                client.clone(),
                vec!["/home/test/remote-source.bin".into()],
                open_local_directory(consumed_path.canonicalize().unwrap()).unwrap(),
                ConflictPolicy::Skip,
            )
            .await
            .unwrap();
        let consumed = consumed_store
            .consume(
                consumed_view.id,
                consumed_view.host_id,
                consumed_view.host_session_id,
                consumed_view.sftp_session_id,
            )
            .unwrap();
        consumed_store
            .discard(
                consumed_view.id,
                consumed_view.host_id,
                consumed_view.host_session_id,
                consumed_view.sftp_session_id,
            )
            .unwrap();
        assert!(std::fs::rename(&consumed_path, parent.path().join("active-plan")).is_err());
        drop(consumed);
        std::fs::rename(&consumed_path, parent.path().join("consume-released")).unwrap();

        let expiring_path = parent.path().join("expiring");
        std::fs::create_dir(&expiring_path).unwrap();
        let expiring_store = FilePlanStore {
            plans: Arc::default(),
            lifetime: Duration::from_secs(10),
        };
        expiring_store
            .plan_download(
                client,
                vec!["/home/test/remote-source.bin".into()],
                open_local_directory(expiring_path.canonicalize().unwrap()).unwrap(),
                ConflictPolicy::Skip,
            )
            .await
            .unwrap();
        assert!(std::fs::rename(&expiring_path, parent.path().join("not-yet")).is_err());
        tokio::time::advance(Duration::from_secs(11)).await;
        tokio::task::yield_now().await;
        assert!(expiring_store.plans.lock().unwrap().is_empty());
        std::fs::rename(&expiring_path, parent.path().join("expiry-released")).unwrap();
    }
}
