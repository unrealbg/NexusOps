use crate::{
    FilePlanStore, InternalPlan, PlanPayload, PlannedDownload, PlannedUpload, Progress, SftpClient,
    policy::{LocalIdentity, local_identity, validate_local_directory, validate_local_source},
};
use nexus_model::{
    AppError, ConflictPolicy, ErrorCode, FileOperationKind, FileOperationPlan, FilePlanItem,
    HostId, HostSessionId, RemoteEntryKind, SftpSessionId, TransferDirection, TransferJob,
    TransferJobId, TransferState,
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tempfile::Builder;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const GLOBAL_ACTIVE: usize = 4;
const HOST_ACTIVE: usize = 2;
const JOB_CAP: usize = 100;
const HISTORY_CAP: usize = 100;

#[derive(Clone)]
struct JobSpec {
    client: Arc<dyn SftpClient>,
    direction: TransferDirection,
    source: Source,
    destination: Destination,
    policy: ConflictPolicy,
    skip: bool,
}
#[derive(Clone)]
enum Source {
    Local(crate::LocalItem),
    Remote(crate::policy::RemoteSource),
}
#[derive(Clone)]
enum Destination {
    Remote {
        path: String,
        expected: Option<crate::EntryIdentity>,
    },
    Local {
        path: PathBuf,
        expected: Option<LocalIdentity>,
    },
}
struct JobRecord {
    view: TransferJob,
    cancel: CancellationToken,
    spec: JobSpec,
}

pub struct TransferManager {
    jobs: Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    order: Arc<Mutex<VecDeque<TransferJobId>>>,
    global: Arc<Semaphore>,
    hosts: Mutex<HashMap<HostId, Arc<Semaphore>>>,
    destinations: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl Default for TransferManager {
    fn default() -> Self {
        Self::new()
    }
}
impl TransferManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            order: Arc::new(Mutex::new(VecDeque::new())),
            global: Arc::new(Semaphore::new(GLOBAL_ACTIVE)),
            hosts: Mutex::new(HashMap::new()),
            destinations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn execute(&self, plan: InternalPlan) -> Result<Vec<TransferJob>, AppError> {
        match plan.payload {
            PlanPayload::Mutation { client, spec } => {
                execute_mutation(client, spec).await?;
                Ok(Vec::new())
            }
            PlanPayload::Upload {
                client,
                policy,
                items,
            } => {
                let mut result = Vec::new();
                for item in items {
                    result.push(
                        self.enqueue(upload_spec(client.clone(), policy, item)?)
                            .await?,
                    );
                }
                Ok(result)
            }
            PlanPayload::Download {
                client,
                policy,
                items,
            } => {
                let mut result = Vec::new();
                for item in items {
                    result.push(
                        self.enqueue(download_spec(client.clone(), policy, item)?)
                            .await?,
                    );
                }
                Ok(result)
            }
        }
    }

    pub fn list(&self, host: Option<HostId>) -> Vec<TransferJob> {
        let Ok(jobs) = self.jobs.lock() else {
            return vec![];
        };
        let Ok(order) = self.order.lock() else {
            return vec![];
        };
        order
            .iter()
            .filter_map(|id| jobs.get(id))
            .filter(|job| host.is_none_or(|value| job.view.host_id == value))
            .map(|job| job.view.clone())
            .collect()
    }

    pub fn cancel(
        &self,
        id: TransferJobId,
        host: HostId,
        host_session: HostSessionId,
        sftp: SftpSessionId,
    ) -> Result<(), AppError> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
        let job = jobs
            .get_mut(&id)
            .ok_or_else(|| transfer_error("The transfer no longer exists."))?;
        if (
            job.view.host_id,
            job.view.host_session_id,
            job.view.sftp_session_id,
        ) != (host, host_session, sftp)
        {
            return Err(AppError::new(
                ErrorCode::Policy,
                "The transfer does not belong to this SFTP session.",
            ));
        }
        if matches!(
            job.view.state,
            TransferState::Queued | TransferState::Preparing | TransferState::Transferring
        ) {
            job.view.state = TransferState::CancelRequested;
            job.cancel.cancel();
        }
        Ok(())
    }

    pub fn retry_plan(
        &self,
        store: &FilePlanStore,
        id: TransferJobId,
        host: HostId,
        host_session: HostSessionId,
        sftp: SftpSessionId,
    ) -> Result<FileOperationPlan, AppError> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
        let job = jobs
            .get(&id)
            .ok_or_else(|| transfer_error("The transfer no longer exists."))?;
        if (
            job.view.host_id,
            job.view.host_session_id,
            job.view.sftp_session_id,
        ) != (host, host_session, sftp)
            || !matches!(
                job.view.state,
                TransferState::Failed | TransferState::Cancelled | TransferState::OutcomeUnknown
            )
        {
            return Err(AppError::new(
                ErrorCode::Policy,
                "Only a failed or cancelled transfer can be prepared for retry.",
            ));
        }
        let payload = match (&job.spec.source, &job.spec.destination) {
            (Source::Local(source), Destination::Remote { path, expected }) => {
                PlanPayload::Upload {
                    client: job.spec.client.clone(),
                    policy: job.spec.policy,
                    items: vec![PlannedUpload {
                        source: source.clone(),
                        destination: path.clone(),
                        expected_destination: expected.clone(),
                        skip: false,
                    }],
                }
            }
            (Source::Remote(source), Destination::Local { path, expected }) => {
                PlanPayload::Download {
                    client: job.spec.client.clone(),
                    policy: job.spec.policy,
                    items: vec![PlannedDownload {
                        source: source.clone(),
                        destination: path.clone(),
                        expected_destination: expected.clone(),
                        skip: false,
                    }],
                }
            }
            _ => return Err(transfer_error("The transfer cannot be retried.")),
        };
        let item = FilePlanItem {
            source_display: job.view.source_display.clone(),
            destination_display: job.view.destination_display.clone(),
            size_bytes: job.view.total_bytes.clone(),
        };
        let kind = if job.view.direction == TransferDirection::Upload {
            FileOperationKind::Upload
        } else {
            FileOperationKind::Download
        };
        store.plan_retry(job.spec.client.info(), payload, item, kind, job.spec.policy)
    }

    pub fn disconnect(&self, host: HostId, host_session: HostSessionId) {
        if let Ok(mut jobs) = self.jobs.lock() {
            for job in jobs
                .values_mut()
                .filter(|job| job.view.host_id == host && job.view.host_session_id == host_session)
            {
                if matches!(
                    job.view.state,
                    TransferState::Queued
                        | TransferState::Preparing
                        | TransferState::Transferring
                        | TransferState::CancelRequested
                ) {
                    job.view.state = TransferState::CancelRequested;
                    job.cancel.cancel();
                }
            }
        }
    }
    pub fn shutdown(&self) {
        if let Ok(mut jobs) = self.jobs.lock() {
            for job in jobs.values_mut() {
                if !matches!(
                    job.view.state,
                    TransferState::Completed
                        | TransferState::Failed
                        | TransferState::Cancelled
                        | TransferState::OutcomeUnknown
                ) {
                    job.view.state = TransferState::CancelRequested;
                    job.cancel.cancel();
                }
            }
        }
    }

    async fn enqueue(&self, spec: JobSpec) -> Result<TransferJob, AppError> {
        let info = spec.client.info();
        let id = TransferJobId::new();
        let (source_display, destination_display, total) = labels(&spec);
        let view = TransferJob {
            id,
            host_id: info.host_id,
            host_session_id: info.host_session_id,
            sftp_session_id: info.id,
            direction: spec.direction,
            source_display,
            destination_display,
            state: TransferState::Queued,
            confirmed_bytes: "0".into(),
            total_bytes: total.map(|v| v.to_string()),
            error: None,
            retryable: false,
        };
        let cancel = CancellationToken::new();
        {
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
            if jobs
                .values()
                .filter(|job| !terminal(&job.view.state))
                .count()
                >= JOB_CAP
            {
                return Err(transfer_error("The transfer queue is full."));
            }
            jobs.insert(
                id,
                JobRecord {
                    view: view.clone(),
                    cancel: cancel.clone(),
                    spec: spec.clone(),
                },
            );
        }
        self.order
            .lock()
            .map_err(|_| transfer_error("The transfer manager is unavailable."))?
            .push_back(id);
        self.trim_history();
        let host_sem = {
            let mut hosts = self
                .hosts
                .lock()
                .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
            hosts
                .entry(info.host_id)
                .or_insert_with(|| Arc::new(Semaphore::new(HOST_ACTIVE)))
                .clone()
        };
        let global = self.global.clone();
        let jobs = self.jobs.clone();
        let destinations = self.destinations.clone();
        tokio::spawn(async move {
            let host_permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => { set_state(&jobs, id, TransferState::Cancelled); return; }
                permit = host_sem.acquire_owned() => match permit {
                    Ok(value) => value,
                    Err(_) => { finish_error(&jobs, id, transfer_error("The transfer queue stopped.")); return; }
                }
            };
            let global_permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => { set_state(&jobs, id, TransferState::Cancelled); return; }
                permit = global.acquire_owned() => match permit {
                    Ok(value) => value,
                    Err(_) => { finish_error(&jobs, id, transfer_error("The transfer queue stopped.")); return; }
                }
            };
            set_state(&jobs, id, TransferState::Preparing);
            let key = destination_key(&spec);
            let lock = {
                let mut map = destinations.lock().expect("destination lock");
                map.entry(key.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                    .clone()
            };
            let destination_guard = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    set_state(&jobs, id, TransferState::Cancelled);
                    if let Ok(mut map) = destinations.lock()
                        && Arc::strong_count(&lock) == 2
                    {
                        map.remove(&key);
                    }
                    return;
                }
                guard = lock.lock() => guard,
            };
            let outcome = run_transfer(&jobs, id, &spec, cancel).await;
            drop(destination_guard);
            drop(global_permit);
            drop(host_permit);
            if let Ok(mut map) = destinations.lock()
                && Arc::strong_count(&lock) == 2
            {
                map.remove(&key);
            }
            match outcome {
                Ok(()) => set_state(&jobs, id, TransferState::Completed),
                Err(error) if error.code == ErrorCode::Cancelled => {
                    set_state(&jobs, id, TransferState::Cancelled)
                }
                Err(error) if error.code == ErrorCode::OutcomeUnknown => {
                    finish_unknown(&jobs, id, error)
                }
                Err(error) => finish_error(&jobs, id, error),
            }
        });
        Ok(view)
    }

    fn trim_history(&self) {
        let Ok(mut order) = self.order.lock() else {
            return;
        };
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        while order.len() > JOB_CAP + HISTORY_CAP {
            if let Some(index) = order
                .iter()
                .position(|id| jobs.get(id).is_some_and(|j| terminal(&j.view.state)))
            {
                if let Some(id) = order.remove(index) {
                    jobs.remove(&id);
                }
            } else {
                break;
            }
        }
    }
}

fn upload_spec(
    client: Arc<dyn SftpClient>,
    policy: ConflictPolicy,
    item: PlannedUpload,
) -> Result<JobSpec, AppError> {
    Ok(JobSpec {
        client,
        direction: TransferDirection::Upload,
        source: Source::Local(item.source),
        destination: Destination::Remote {
            path: item.destination,
            expected: item.expected_destination,
        },
        policy,
        skip: item.skip,
    })
}
fn download_spec(
    client: Arc<dyn SftpClient>,
    policy: ConflictPolicy,
    item: PlannedDownload,
) -> Result<JobSpec, AppError> {
    Ok(JobSpec {
        client,
        direction: TransferDirection::Download,
        source: Source::Remote(item.source),
        destination: Destination::Local {
            path: item.destination,
            expected: item.expected_destination,
        },
        policy,
        skip: item.skip,
    })
}
fn labels(spec: &JobSpec) -> (String, String, Option<u64>) {
    match (&spec.source, &spec.destination) {
        (Source::Local(s), Destination::Remote { path, .. }) => (
            crate::display_name(&s.display_name),
            crate::display_name(path),
            Some(s.size),
        ),
        (Source::Remote(s), Destination::Local { path, .. }) => (
            crate::display_name(&s.name),
            path.file_name()
                .and_then(|v| v.to_str())
                .map(crate::display_name)
                .unwrap_or_else(|| "Unavailable".into()),
            s.size,
        ),
        _ => ("Unavailable".into(), "Unavailable".into(), None),
    }
}
fn destination_key(spec: &JobSpec) -> String {
    match &spec.destination {
        Destination::Remote { path, .. } => format!("r:{}:{path}", spec.client.info().id.0),
        Destination::Local { path, .. } => format!("l:{}", path.to_string_lossy().to_lowercase()),
    }
}

async fn run_transfer(
    jobs: &Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    id: TransferJobId,
    spec: &JobSpec,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    if spec.skip {
        return Ok(());
    }
    match (&spec.source, &spec.destination) {
        (Source::Local(source), destination @ Destination::Remote { .. }) => {
            run_upload(jobs, id, spec, source, destination, cancel).await
        }
        (Source::Remote(source), destination @ Destination::Local { .. }) => {
            run_download(jobs, id, spec, source, destination, cancel).await
        }
        _ => Err(transfer_error("Invalid transfer specification.")),
    }
}

async fn run_upload(
    jobs: &Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    id: TransferJobId,
    spec: &JobSpec,
    source: &crate::LocalItem,
    destination: &Destination,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    let Destination::Remote {
        path: destination,
        expected,
    } = destination
    else {
        return Err(transfer_error("Invalid upload destination."));
    };
    let client = spec.client.clone();
    validate_local_source(source)?;
    let mut file = tokio::fs::File::open(&source.path).await.map_err(|_| {
        AppError::new(
            ErrorCode::LocalAccess,
            "The selected local source could not be opened.",
        )
    })?;
    let parent = crate::parent_remote(destination)?;
    let staging = crate::join_remote(&parent, &format!(".nexusops-{}.part", id.0.simple()))?;
    let progress = progress(jobs.clone(), id);
    set_state(jobs, id, TransferState::Transferring);
    let streamed = match client
        .upload_staged(&mut file, &staging, cancel.clone(), progress)
        .await
    {
        Ok(value) => value,
        Err(error) => {
            client.remove_owned_staging(&staging).await;
            return Err(error);
        }
    };
    if streamed != source.size {
        client.remove_owned_staging(&staging).await;
        return Err(transfer_error(
            "The local source size changed during transfer.",
        ));
    }
    if cancel.is_cancelled() {
        client.remove_owned_staging(&staging).await;
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The transfer was cancelled before finalization.",
        ));
    }
    set_state(jobs, id, TransferState::Finalizing);
    if client.identity(destination).await? != *expected {
        client.remove_owned_staging(&staging).await;
        return Err(AppError::new(
            ErrorCode::Conflict,
            "The remote destination changed after approval.",
        ));
    }
    let result = if spec.policy == ConflictPolicy::Replace {
        client.commit_replace(&staging, destination).await
    } else {
        client.commit_new(&staging, destination).await
    };
    if result.is_err() {
        client.remove_owned_staging(&staging).await;
    }
    result
}

async fn run_download(
    jobs: &Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    id: TransferJobId,
    spec: &JobSpec,
    source: &crate::policy::RemoteSource,
    destination: &Destination,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    let Destination::Local {
        path: destination,
        expected,
    } = destination
    else {
        return Err(transfer_error("Invalid download destination."));
    };
    let client = spec.client.clone();
    if client.identity(&source.path).await? != Some(source.identity.clone()) {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "The remote source changed after approval.",
        ));
    }
    let directory = destination.parent().ok_or_else(|| {
        AppError::new(
            ErrorCode::LocalAccess,
            "The local destination has no parent directory.",
        )
    })?;
    validate_local_directory(directory)?;
    let named = Builder::new()
        .prefix(".nexusops-")
        .suffix(".part")
        .tempfile_in(directory)
        .map_err(|_| {
            AppError::new(
                ErrorCode::LocalAccess,
                "A local staging file could not be created.",
            )
        })?;
    let temp = named.into_temp_path();
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&temp)
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::LocalAccess,
                "The local staging file could not be opened.",
            )
        })?;
    set_state(jobs, id, TransferState::Transferring);
    let bytes = client
        .download(
            &source.path,
            &mut file,
            cancel.clone(),
            progress(jobs.clone(), id),
        )
        .await?;
    if source.size.is_some_and(|size| size != bytes) {
        return Err(transfer_error(
            "The remote source size changed during transfer.",
        ));
    }
    file.sync_all()
        .await
        .map_err(|_| transfer_error("The local staging file could not be synchronized."))?;
    drop(file);
    if cancel.is_cancelled() {
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The transfer was cancelled before finalization.",
        ));
    }
    set_state(jobs, id, TransferState::Finalizing);
    if local_identity(destination)? != *expected {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "The local destination changed after approval.",
        ));
    }
    match spec.policy {
        ConflictPolicy::Replace => temp.persist(destination).map_err(|_| {
            AppError::new(
                ErrorCode::OutcomeUnknown,
                "The local Replace result could not be confirmed.",
            )
        })?,
        _ => temp.persist_noclobber(destination).map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "The local destination appeared before finalization.",
            )
        })?,
    };
    Ok(())
}

async fn execute_mutation(
    client: Arc<dyn SftpClient>,
    spec: crate::MutationSpec,
) -> Result<(), AppError> {
    match spec {
        crate::MutationSpec::CreateDirectory { path } => client.create_dir(&path).await,
        crate::MutationSpec::Rename {
            source,
            destination,
        } => client.rename_noclobber(&source, &destination).await,
        crate::MutationSpec::Delete {
            path,
            kind,
            expected,
        } => {
            if client.identity(&path).await? != Some(expected) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The remote entry changed after approval.",
                ));
            }
            match kind {
                RemoteEntryKind::File | RemoteEntryKind::Symlink => client.remove_file(&path).await,
                RemoteEntryKind::Directory => client.remove_dir(&path).await,
                _ => Err(AppError::new(
                    ErrorCode::FilePolicy,
                    "Special files cannot be deleted.",
                )),
            }
        }
    }
}
fn progress(jobs: Arc<Mutex<HashMap<TransferJobId, JobRecord>>>, id: TransferJobId) -> Progress {
    Arc::new(move |bytes| {
        if let Ok(mut map) = jobs.lock()
            && let Some(job) = map.get_mut(&id)
        {
            job.view.confirmed_bytes = bytes.to_string();
        }
    })
}
fn set_state(
    jobs: &Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    id: TransferJobId,
    state: TransferState,
) {
    if let Ok(mut map) = jobs.lock()
        && let Some(job) = map.get_mut(&id)
    {
        job.view.state = state;
    }
}
fn finish_error(
    jobs: &Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    id: TransferJobId,
    error: AppError,
) {
    if let Ok(mut map) = jobs.lock()
        && let Some(job) = map.get_mut(&id)
    {
        job.view.state = TransferState::Failed;
        job.view.error = Some(error);
        job.view.retryable = matches!(
            job.view.error.as_ref().map(|value| value.code),
            Some(
                ErrorCode::Timeout
                    | ErrorCode::Connection
                    | ErrorCode::LocalAccess
                    | ErrorCode::Transfer
            )
        );
    }
}
fn finish_unknown(
    jobs: &Arc<Mutex<HashMap<TransferJobId, JobRecord>>>,
    id: TransferJobId,
    error: AppError,
) {
    if let Ok(mut map) = jobs.lock()
        && let Some(job) = map.get_mut(&id)
    {
        job.view.state = TransferState::OutcomeUnknown;
        job.view.error = Some(error);
        job.view.retryable = false;
    }
}
fn terminal(state: &TransferState) -> bool {
    matches!(
        state,
        TransferState::Completed
            | TransferState::Failed
            | TransferState::Cancelled
            | TransferState::OutcomeUnknown
    )
}
fn transfer_error(message: &'static str) -> AppError {
    AppError::new(ErrorCode::Transfer, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nexus_model::{DirectoryListing, RemoteEntry, SftpLimits, SftpSessionInfo};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

    struct CountingClient {
        info: SftpSessionInfo,
        upload_calls: AtomicUsize,
    }

    impl CountingClient {
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
                        client_chunk_bytes: "3".into(),
                        listing_entry_cap: 5_000,
                    },
                    root_path: "/tmp".into(),
                },
                upload_calls: AtomicUsize::new(0),
            })
        }
    }

    fn unused() -> AppError {
        transfer_error("Unused mock operation.")
    }

    #[async_trait]
    impl SftpClient for CountingClient {
        fn info(&self) -> SftpSessionInfo {
            self.info.clone()
        }
        async fn list(&self, _: &str, _: CancellationToken) -> Result<DirectoryListing, AppError> {
            Err(unused())
        }
        async fn stat(&self, _: &str) -> Result<RemoteEntry, AppError> {
            Err(unused())
        }
        async fn identity(&self, _: &str) -> Result<Option<crate::EntryIdentity>, AppError> {
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
            reader: &mut (dyn AsyncRead + Unpin + Send),
            _: &str,
            cancel: CancellationToken,
            progress: Progress,
        ) -> Result<u64, AppError> {
            self.upload_calls.fetch_add(1, Ordering::SeqCst);
            let mut bytes = 0_u64;
            let mut buffer = [0_u8; 3];
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(AppError::new(ErrorCode::Cancelled, "Cancelled.")),
                    result = reader.read(&mut buffer) => {
                        let count = result.map_err(|_| unused())?;
                        if count == 0 { break; }
                        bytes = bytes.checked_add(count as u64).ok_or_else(unused)?;
                        progress(bytes);
                    }
                }
            }
            Ok(bytes)
        }
        async fn download(
            &self,
            _: &str,
            _: &mut (dyn AsyncWrite + Unpin + Send),
            _: CancellationToken,
            _: Progress,
        ) -> Result<u64, AppError> {
            Err(unused())
        }
        async fn commit_new(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn commit_replace(&self, _: &str, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn remove_owned_staging(&self, _: &str) {}
        async fn close(&self) {}
    }

    fn local_item(path: &std::path::Path) -> crate::LocalItem {
        let metadata = std::fs::metadata(path).unwrap();
        crate::LocalItem {
            path: path.to_path_buf(),
            display_name: "binary.dat".into(),
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }

    #[tokio::test]
    async fn skip_policy_only_skips_items_that_actually_conflicted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("binary.dat");
        std::fs::write(&path, [0_u8, 255, 1, 2, 3, 4, 5]).unwrap();
        let client = CountingClient::new();
        let planned = PlannedUpload {
            source: local_item(&path),
            destination: "/tmp/binary.dat".into(),
            expected_destination: None,
            skip: false,
        };
        let spec = upload_spec(client.clone(), ConflictPolicy::Skip, planned).unwrap();
        run_transfer(
            &Arc::new(Mutex::new(HashMap::new())),
            TransferJobId::new(),
            &spec,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(client.upload_calls.load(Ordering::SeqCst), 1);

        let skipped = PlannedUpload {
            source: local_item(&path),
            destination: "/tmp/binary.dat".into(),
            expected_destination: Some(crate::EntryIdentity {
                kind: RemoteEntryKind::File,
                size: Some(7),
                modified: None,
            }),
            skip: true,
        };
        let skipped_spec = upload_spec(client.clone(), ConflictPolicy::Skip, skipped).unwrap();
        run_transfer(
            &Arc::new(Mutex::new(HashMap::new())),
            TransferJobId::new(),
            &skipped_spec,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(client.upload_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_stream_never_reaches_finalization() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("binary.dat");
        std::fs::write(&path, [1_u8; 9]).unwrap();
        let client = CountingClient::new();
        let spec = upload_spec(
            client,
            ConflictPolicy::KeepBoth,
            PlannedUpload {
                source: local_item(&path),
                destination: "/tmp/cancel.dat".into(),
                expected_destination: None,
                skip: false,
            },
        )
        .unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(HashMap::new())),
                TransferJobId::new(),
                &spec,
                cancel
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Cancelled
        );
    }

    #[tokio::test]
    async fn queued_cancellation_starts_no_remote_io() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.dat");
        std::fs::write(&path, [7_u8; 32]).unwrap();
        let client = CountingClient::new();
        let manager = TransferManager::new();
        let held_global = manager
            .global
            .clone()
            .acquire_many_owned(GLOBAL_ACTIVE as u32)
            .await
            .unwrap();
        let job = manager
            .enqueue(
                upload_spec(
                    client.clone(),
                    ConflictPolicy::KeepBoth,
                    PlannedUpload {
                        source: local_item(&path),
                        destination: "/tmp/queued.dat".into(),
                        expected_destination: None,
                        skip: false,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let info = client.info();
        manager
            .cancel(job.id, info.host_id, info.host_session_id, info.id)
            .unwrap();
        for _ in 0..50 {
            if manager.list(None)[0].state == TransferState::Cancelled {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(manager.list(None)[0].state, TransferState::Cancelled);
        assert_eq!(client.upload_calls.load(Ordering::SeqCst), 0);
        drop(held_global);
    }
}
