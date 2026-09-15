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
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

const PLAN_LIFETIME: Duration = Duration::from_secs(5 * 60);
const PLAN_CAP: usize = 128;

#[derive(Clone, Debug)]
pub struct LocalItem {
    pub path: PathBuf,
    pub display_name: String,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
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
    pub expected_destination: Option<EntryIdentity>,
    pub skip: bool,
}

#[derive(Clone, Debug)]
pub struct PlannedDownload {
    pub source: RemoteSource,
    pub destination: PathBuf,
    pub expected_destination: Option<LocalIdentity>,
    pub skip: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalIdentity {
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug)]
pub enum MutationSpec {
    CreateDirectory {
        path: String,
    },
    Rename {
        source: String,
        destination: String,
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

#[derive(Default)]
pub struct FilePlanStore {
    plans: Mutex<HashMap<FilePlanId, StoredPlan>>,
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
            validate_child_name(&source.display_name)?;
            let base = join_remote(remote_directory, &source.display_name)?;
            let existing = client.identity(&base).await?;
            let (destination, expected, skip) = match (existing, policy) {
                (None, _) if supports(&client.info(), "hardlink@openssh.com", "1") => {
                    (base, None, false)
                }
                (None, _) => {
                    return Err(policy_error(
                        "Safe new-file finalization requires hardlink@openssh.com v1.",
                    ));
                }
                (Some(identity), ConflictPolicy::Skip) => (base, Some(identity), true),
                (Some(identity), ConflictPolicy::Replace)
                    if identity.kind == RemoteEntryKind::File
                        && supports(&client.info(), "posix-rename@openssh.com", "1") =>
                {
                    (base, Some(identity), false)
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
                    (candidate, None, false)
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
                expected_destination: expected,
                skip,
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
        local_directory: PathBuf,
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
            let mut destination = local_directory.join(&entry.name);
            let existing = local_identity(&destination)?;
            let (expected, skip) = match (existing, policy) {
                (None, _) => (None, false),
                (Some(value), ConflictPolicy::Skip) => (Some(value), true),
                (Some(value), ConflictPolicy::Replace) => (Some(value), false),
                (Some(_), ConflictPolicy::KeepBoth) => {
                    destination = keep_both_local(&local_directory, &entry.name)?;
                    (None, false)
                }
            };
            let identity = client.identity(&path).await?.ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "The remote source no longer exists.")
            })?;
            items.push(FilePlanItem {
                source_display: entry.display_name,
                destination_display: destination
                    .file_name()
                    .and_then(|v| v.to_str())
                    .map(display_name)
                    .unwrap_or_else(|| "Unavailable".into()),
                size_bytes: entry.size_bytes,
            });
            planned.push(PlannedDownload {
                source: RemoteSource {
                    path,
                    name: entry.name,
                    size: identity.size,
                    identity,
                },
                destination,
                expected_destination: expected,
                skip,
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

    pub fn plan_retry(
        &self,
        info: nexus_model::SftpSessionInfo,
        payload: PlanPayload,
        item: FilePlanItem,
        kind: FileOperationKind,
        policy: ConflictPolicy,
    ) -> Result<FileOperationPlan, AppError> {
        self.insert(
            info,
            kind,
            OperationRisk::Moderate,
            Some(policy),
            vec![item],
            payload,
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
        let stored = plans.get_mut(&id).ok_or_else(|| {
            policy_error("The file-operation plan is missing or was already used.")
        })?;
        if Instant::now() > stored.expires {
            stored.plan = None;
            return Err(policy_error("The file-operation plan expired."));
        }
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
        let expires = Instant::now() + PLAN_LIFETIME;
        let view = FileOperationPlan {
            id,
            host_id: info.host_id,
            host_session_id: info.host_session_id,
            sftp_session_id: info.id,
            kind,
            risk,
            conflict_policy,
            items,
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
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
        Ok(view)
    }
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
            Ok(Some(LocalIdentity {
                size: metadata.len(),
                modified: metadata.modified().ok(),
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(AppError::new(
            ErrorCode::LocalAccess,
            "The local destination metadata could not be read.",
        )),
    }
}

pub fn validate_local_directory(path: &std::path::Path) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        AppError::new(
            ErrorCode::LocalAccess,
            "The selected local directory is unavailable.",
        )
    })?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(policy_error(
            "The selected local directory must not be a symlink or reparse point.",
        ));
    }
    Ok(())
}

pub fn validate_local_source(item: &LocalItem) -> Result<(), AppError> {
    let metadata = std::fs::symlink_metadata(&item.path).map_err(|_| {
        AppError::new(
            ErrorCode::LocalAccess,
            "The selected local source is unavailable.",
        )
    })?;
    if !metadata.file_type().is_file()
        || is_reparse(&metadata)
        || metadata.len() != item.size
        || metadata.modified().ok() != item.modified
    {
        return Err(policy_error(
            "The selected local source changed after approval.",
        ));
    }
    Ok(())
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
            Arc::new(Self {
                info: SftpSessionInfo {
                    id: SftpSessionId::new(),
                    host_id: HostId::new(),
                    host_session_id: HostSessionId::new(),
                    protocol_version: 3,
                    extensions: Vec::new(),
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
            Err(unused())
        }
        async fn identity(&self, _: &str) -> Result<Option<EntryIdentity>, AppError> {
            Ok(None)
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
            _: CancellationToken,
            _: crate::Progress,
        ) -> Result<u64, AppError> {
            Err(unused())
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
        let error = store
            .plan_upload(
                client,
                vec![LocalItem {
                    path: PathBuf::from("opaque-test-path"),
                    display_name: "data.bin".into(),
                    size: 1,
                    modified: None,
                }],
                "/home/test",
                ConflictPolicy::Skip,
            )
            .await
            .expect_err("unsafe server must be rejected before approval");
        assert_eq!(error.code, ErrorCode::FilePolicy);
    }
}
