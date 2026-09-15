use crate::{
    DestinationAction, FilePlanStore, InternalPlan, PlanPayload, PlannedDownload, PlannedUpload,
    Progress, SftpClient, StagedUploadFailure, StagingOwnership,
    policy::{
        LocalIdentity, local_identity, validate_local_directory_handle, validate_local_source,
    },
};
use nexus_model::{
    AppError, ConflictPolicy, ErrorCode, FileOperationPlan, HostId, HostSessionId, RemoteEntryKind,
    SftpSessionId, TransferDirection, TransferJob, TransferJobId, TransferState,
};
use std::{
    collections::{HashMap, VecDeque},
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
        action: DestinationAction<crate::EntryIdentity>,
    },
    Local {
        directory: crate::LocalDirectory,
        file_name: String,
        action: DestinationAction<LocalIdentity>,
    },
}
struct JobRecord {
    view: TransferJob,
    cancel: CancellationToken,
    /// Present only while the job is active. Terminal history never retains
    /// selected local handles or remote clients.
    active_spec: Option<JobSpec>,
}
struct RemoteStagingGuard {
    client: Arc<dyn SftpClient>,
    path: String,
    ownership: StagingOwnership,
    armed: bool,
}

impl RemoteStagingGuard {
    fn new(client: Arc<dyn SftpClient>, path: String, ownership: StagingOwnership) -> Self {
        Self {
            client,
            path,
            ownership,
            armed: true,
        }
    }
    async fn cleanup(&mut self) {
        if self.armed && self.ownership.is_owned() {
            self.client.remove_owned_staging(&self.path).await;
        }
        self.armed = false;
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RemoteStagingGuard {
    fn drop(&mut self) {
        if !self.armed || !self.ownership.is_owned() {
            return;
        }
        let client = self.client.clone();
        let path = self.path.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                client.remove_owned_staging(&path).await;
            });
        }
    }
}
#[derive(Default)]
struct ManagerState {
    jobs: HashMap<TransferJobId, JobRecord>,
    order: VecDeque<TransferJobId>,
}

pub struct TransferManager {
    state: Arc<Mutex<ManagerState>>,
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
            state: Arc::new(Mutex::new(ManagerState::default())),
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
                let specs = items
                    .into_iter()
                    .map(|item| upload_spec(client.clone(), policy, item))
                    .collect::<Result<Vec<_>, _>>()?;
                self.enqueue_batch(specs).await
            }
            PlanPayload::Download {
                client,
                policy,
                items,
            } => {
                let specs = items
                    .into_iter()
                    .map(|item| download_spec(client.clone(), policy, item))
                    .collect::<Result<Vec<_>, _>>()?;
                self.enqueue_batch(specs).await
            }
        }
    }

    pub fn list(&self, host: Option<HostId>) -> Vec<TransferJob> {
        let Ok(state) = self.state.lock() else {
            return vec![];
        };
        state
            .order
            .iter()
            .filter_map(|id| state.jobs.get(id))
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
        let mut state = self
            .state
            .lock()
            .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
        let job = state
            .jobs
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

    pub async fn retry_plan(
        &self,
        _store: &FilePlanStore,
        id: TransferJobId,
        host: HostId,
        host_session: HostSessionId,
        sftp: SftpSessionId,
    ) -> Result<FileOperationPlan, AppError> {
        {
            let state = self
                .state
                .lock()
                .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
            let job = state
                .jobs
                .get(&id)
                .ok_or_else(|| transfer_error("The transfer no longer exists."))?;
            if (
                job.view.host_id,
                job.view.host_session_id,
                job.view.sftp_session_id,
            ) != (host, host_session, sftp)
            {
                return Err(AppError::new(
                    ErrorCode::Policy,
                    "The transfer belongs to a different SFTP session.",
                ));
            }
            let allowed = job.view.state == TransferState::Cancelled
                || (job.view.state == TransferState::Failed && job.view.retryable);
            if !allowed {
                return Err(AppError::new(
                    ErrorCode::Policy,
                    "This transfer outcome cannot be retried; refresh and prepare a new operation.",
                ));
            }
        }
        Err(AppError::new(
            ErrorCode::Policy,
            "Select the local source or destination again to prepare a retry.",
        ))
    }

    pub fn disconnect(&self, host: HostId, host_session: HostSessionId) {
        if let Ok(mut state) = self.state.lock() {
            for job in state
                .jobs
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
        if let Ok(mut state) = self.state.lock() {
            for job in state.jobs.values_mut() {
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

    #[cfg(test)]
    async fn enqueue(&self, spec: JobSpec) -> Result<TransferJob, AppError> {
        self.enqueue_batch(vec![spec])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| transfer_error("The transfer batch was empty."))
    }

    async fn enqueue_batch(&self, specs: Vec<JobSpec>) -> Result<Vec<TransferJob>, AppError> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        let host_sems = {
            let mut hosts = self
                .hosts
                .lock()
                .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
            specs
                .iter()
                .map(|spec| {
                    hosts
                        .entry(spec.client.info().host_id)
                        .or_insert_with(|| Arc::new(Semaphore::new(HOST_ACTIVE)))
                        .clone()
                })
                .collect::<Vec<_>>()
        };
        let prepared = specs
            .into_iter()
            .zip(host_sems)
            .map(|(spec, host_sem)| {
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
                    total_bytes: total.map(|value| value.to_string()),
                    error: None,
                    retryable: false,
                };
                (id, view, CancellationToken::new(), spec, host_sem)
            })
            .collect::<Vec<_>>();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| transfer_error("The transfer manager is unavailable."))?;
            trim_history_locked(&mut state);
            let nonterminal = state
                .jobs
                .values()
                .filter(|job| !terminal(&job.view.state))
                .count();
            if prepared.len() > JOB_CAP.saturating_sub(nonterminal) {
                return Err(transfer_error(
                    "The complete transfer batch cannot fit in the bounded queue.",
                ));
            }
            for (id, view, cancel, spec, _) in &prepared {
                state.order.push_back(*id);
                state.jobs.insert(
                    *id,
                    JobRecord {
                        view: view.clone(),
                        cancel: cancel.clone(),
                        active_spec: Some(spec.clone()),
                    },
                );
            }
        }
        let views = prepared
            .iter()
            .map(|(_, view, _, _, _)| view.clone())
            .collect();
        for (id, _, cancel, spec, host_sem) in prepared {
            self.spawn_job(id, cancel, spec, host_sem);
        }
        Ok(views)
    }

    fn spawn_job(
        &self,
        id: TransferJobId,
        cancel: CancellationToken,
        spec: JobSpec,
        host_sem: Arc<Semaphore>,
    ) {
        let global = self.global.clone();
        let state = self.state.clone();
        let destinations = self.destinations.clone();
        tokio::spawn(async move {
            let host_permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    drop(spec);
                    set_state(&state, id, TransferState::Cancelled);
                    return;
                }
                permit = host_sem.acquire_owned() => match permit {
                    Ok(value) => value,
                    Err(_) => {
                        drop(spec);
                        finish_error(&state, id, transfer_error("The transfer queue stopped."));
                        return;
                    }
                }
            };
            let global_permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    drop(spec);
                    set_state(&state, id, TransferState::Cancelled);
                    return;
                }
                permit = global.acquire_owned() => match permit {
                    Ok(value) => value,
                    Err(_) => {
                        drop(spec);
                        finish_error(&state, id, transfer_error("The transfer queue stopped."));
                        return;
                    }
                }
            };
            set_state(&state, id, TransferState::Preparing);
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
                    drop(spec);
                    set_state(&state, id, TransferState::Cancelled);
                    if let Ok(mut map) = destinations.lock()
                        && Arc::strong_count(&lock) == 2
                    {
                        map.remove(&key);
                    }
                    return;
                }
                guard = lock.lock() => guard,
            };
            let outcome = run_transfer(&state, id, &spec, cancel).await;
            drop(destination_guard);
            drop(global_permit);
            drop(host_permit);
            if let Ok(mut map) = destinations.lock()
                && Arc::strong_count(&lock) == 2
            {
                map.remove(&key);
            }
            drop(spec);
            match outcome {
                Ok(()) => set_state(&state, id, TransferState::Completed),
                Err(error) if error.code == ErrorCode::Cancelled => {
                    set_state(&state, id, TransferState::Cancelled)
                }
                Err(error) if error.code == ErrorCode::OutcomeUnknown => {
                    finish_unknown(&state, id, error)
                }
                Err(error) => finish_error(&state, id, error),
            }
        });
    }
}

fn trim_history_locked(state: &mut ManagerState) {
    while state
        .jobs
        .values()
        .filter(|job| terminal(&job.view.state))
        .count()
        > HISTORY_CAP
    {
        let Some(index) = state.order.iter().position(|id| {
            state
                .jobs
                .get(id)
                .is_some_and(|job| terminal(&job.view.state))
        }) else {
            break;
        };
        if let Some(id) = state.order.remove(index) {
            state.jobs.remove(&id);
        }
    }
}

fn upload_spec(
    client: Arc<dyn SftpClient>,
    _policy: ConflictPolicy,
    item: PlannedUpload,
) -> Result<JobSpec, AppError> {
    Ok(JobSpec {
        client,
        direction: TransferDirection::Upload,
        source: Source::Local(item.source),
        destination: Destination::Remote {
            path: item.destination,
            action: item.action,
        },
    })
}
fn download_spec(
    client: Arc<dyn SftpClient>,
    _policy: ConflictPolicy,
    item: PlannedDownload,
) -> Result<JobSpec, AppError> {
    Ok(JobSpec {
        client,
        direction: TransferDirection::Download,
        source: Source::Remote(item.source),
        destination: Destination::Local {
            directory: item.destination,
            file_name: item.file_name,
            action: item.action,
        },
    })
}
fn labels(spec: &JobSpec) -> (String, String, Option<u64>) {
    match (&spec.source, &spec.destination) {
        (Source::Local(s), Destination::Remote { path, .. }) => (
            crate::display_name(&s.display_name),
            crate::display_name(path),
            Some(s.size),
        ),
        (
            Source::Remote(s),
            Destination::Local {
                directory,
                file_name,
                ..
            },
        ) => (
            crate::display_name(&s.name),
            crate::display_name(&format!("{}\\{}", directory.display_name, file_name)),
            s.size,
        ),
        _ => ("Unavailable".into(), "Unavailable".into(), None),
    }
}
fn destination_key(spec: &JobSpec) -> String {
    match &spec.destination {
        Destination::Remote { path, .. } => format!("r:{}:{path}", spec.client.info().id.0),
        Destination::Local {
            directory,
            file_name,
            ..
        } => format!(
            "l:{}:{}:{}",
            directory.identity.object_id.storage_id,
            directory.identity.object_id.file_id,
            file_name.to_lowercase()
        ),
    }
}

async fn run_transfer(
    state: &Arc<Mutex<ManagerState>>,
    id: TransferJobId,
    spec: &JobSpec,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    if matches!(
        &spec.destination,
        Destination::Remote {
            action: DestinationAction::Skip,
            ..
        } | Destination::Local {
            action: DestinationAction::Skip,
            ..
        }
    ) {
        return Ok(());
    }
    match (&spec.source, &spec.destination) {
        (Source::Local(source), destination @ Destination::Remote { .. }) => {
            run_upload(state, id, spec, source, destination, cancel).await
        }
        (Source::Remote(source), destination @ Destination::Local { .. }) => {
            run_download(state, id, spec, source, destination, cancel).await
        }
        _ => Err(transfer_error("Invalid transfer specification.")),
    }
}

async fn run_upload(
    state: &Arc<Mutex<ManagerState>>,
    id: TransferJobId,
    spec: &JobSpec,
    source: &crate::LocalItem,
    destination: &Destination,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    let Destination::Remote {
        path: destination,
        action,
    } = destination
    else {
        return Err(transfer_error("Invalid upload destination."));
    };
    let client = spec.client.clone();
    validate_local_source(source)?;
    let source_handle = source.handle.try_clone().map_err(|_| {
        AppError::new(
            ErrorCode::LocalAccess,
            "The selected local source handle is unavailable.",
        )
    })?;
    let mut file = tokio::fs::File::from_std(source_handle);
    tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(0))
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::LocalAccess,
                "The selected local source could not be read from the beginning.",
            )
        })?;
    let parent = crate::parent_remote(destination)?;
    let staging = crate::join_remote(&parent, &format!(".nexusops-{}.part", id.0.simple()))?;
    let ownership = StagingOwnership::new();
    let mut staging_guard =
        RemoteStagingGuard::new(client.clone(), staging.clone(), ownership.clone());
    let progress = progress(state.clone(), id);
    set_state(state, id, TransferState::Transferring);
    let streamed = match client
        .upload_staged(&mut file, &staging, ownership, cancel.clone(), progress)
        .await
    {
        Ok(value) => value,
        Err(StagedUploadFailure::NotCreated(error)) => return Err(error),
        Err(StagedUploadFailure::CreationOutcomeUnknown(error)) => return Err(error),
        Err(StagedUploadFailure::OwnedFailure(error)) => {
            staging_guard.cleanup().await;
            return Err(error);
        }
    };
    if streamed != source.size {
        staging_guard.cleanup().await;
        return Err(transfer_error(
            "The local source size changed during transfer.",
        ));
    }
    if let Err(error) = validate_local_source(source) {
        staging_guard.cleanup().await;
        return Err(error);
    }
    if cancel.is_cancelled() {
        staging_guard.cleanup().await;
        return Err(AppError::new(
            ErrorCode::Cancelled,
            "The transfer was cancelled before finalization.",
        ));
    }
    set_state(state, id, TransferState::Finalizing);
    let current = match client.identity(destination).await {
        Ok(value) => value,
        Err(error) => {
            staging_guard.cleanup().await;
            return Err(error);
        }
    };
    let destination_matches = match action {
        DestinationAction::CreateNew => current.is_none(),
        DestinationAction::ReplaceExisting(expected) => current.as_ref() == Some(expected),
        DestinationAction::Skip => true,
    };
    if !destination_matches {
        staging_guard.cleanup().await;
        return Err(AppError::new(
            ErrorCode::Conflict,
            "The remote destination changed after approval.",
        ));
    }
    let result = match action {
        DestinationAction::ReplaceExisting(_) => client.commit_replace(&staging, destination).await,
        DestinationAction::CreateNew => client.commit_new(&staging, destination).await,
        DestinationAction::Skip => Ok(()),
    };
    if result.is_err() {
        staging_guard.cleanup().await;
    } else {
        staging_guard.disarm();
    }
    result
}

async fn run_download(
    state: &Arc<Mutex<ManagerState>>,
    id: TransferJobId,
    spec: &JobSpec,
    source: &crate::policy::RemoteSource,
    destination: &Destination,
    cancel: CancellationToken,
) -> Result<(), AppError> {
    let Destination::Local {
        directory,
        file_name,
        action,
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
    validate_local_directory_handle(directory)?;
    let destination = directory.path.join(file_name);
    let named = Builder::new()
        .prefix(".nexusops-")
        .suffix(".part")
        .tempfile_in(&directory.path)
        .map_err(|_| {
            AppError::new(
                ErrorCode::LocalAccess,
                "A local staging file could not be created.",
            )
        })?;
    let (staging_file, temp) = named.into_parts();
    let mut file = tokio::fs::File::from_std(staging_file);
    set_state(state, id, TransferState::Transferring);
    let bytes = client
        .download(
            &source.path,
            &mut file,
            cancel.clone(),
            progress(state.clone(), id),
        )
        .await?;
    if source.size.is_some_and(|size| size != bytes) {
        return Err(transfer_error(
            "The remote source size changed during transfer.",
        ));
    }
    if client.identity(&source.path).await? != Some(source.identity.clone()) {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "The remote source changed during transfer.",
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
    set_state(state, id, TransferState::Finalizing);
    validate_local_directory_handle(directory)?;
    let current = local_identity(&destination)?;
    let destination_matches = match action {
        DestinationAction::CreateNew => current.is_none(),
        DestinationAction::ReplaceExisting(expected) => current.as_ref() == Some(expected),
        DestinationAction::Skip => true,
    };
    if !destination_matches {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "The local destination changed after approval.",
        ));
    }
    match action {
        DestinationAction::ReplaceExisting(_) => temp.persist(&destination).map_err(|_| {
            AppError::new(
                ErrorCode::OutcomeUnknown,
                "The local Replace result could not be confirmed.",
            )
        })?,
        DestinationAction::CreateNew => temp.persist_noclobber(&destination).map_err(|_| {
            AppError::new(
                ErrorCode::Conflict,
                "The local destination appeared before finalization.",
            )
        })?,
        DestinationAction::Skip => return Ok(()),
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
            expected_source,
        } => {
            if client.identity(&source).await? != Some(expected_source) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "The remote source changed after approval.",
                ));
            }
            client.rename_noclobber(&source, &destination).await
        }
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
fn progress(state: Arc<Mutex<ManagerState>>, id: TransferJobId) -> Progress {
    Arc::new(move |bytes| {
        if let Ok(mut value) = state.lock()
            && let Some(job) = value.jobs.get_mut(&id)
        {
            job.view.confirmed_bytes = bytes.to_string();
        }
    })
}
fn set_state(manager: &Arc<Mutex<ManagerState>>, id: TransferJobId, next: TransferState) {
    if let Ok(mut value) = manager.lock() {
        let should_trim = if let Some(job) = value.jobs.get_mut(&id) {
            job.view.state = next;
            let terminal = terminal(&job.view.state);
            if terminal {
                job.active_spec = None;
                job.view.retryable = false;
            }
            terminal
        } else {
            false
        };
        if should_trim {
            trim_history_locked(&mut value);
        }
    }
}
fn finish_error(state: &Arc<Mutex<ManagerState>>, id: TransferJobId, error: AppError) {
    if let Ok(mut value) = state.lock() {
        if let Some(job) = value.jobs.get_mut(&id) {
            job.view.state = TransferState::Failed;
            job.view.error = Some(error);
            job.view.retryable = false;
            job.active_spec = None;
        }
        trim_history_locked(&mut value);
    }
}
fn finish_unknown(state: &Arc<Mutex<ManagerState>>, id: TransferJobId, error: AppError) {
    if let Ok(mut value) = state.lock() {
        if let Some(job) = value.jobs.get_mut(&id) {
            job.view.state = TransferState::OutcomeUnknown;
            job.view.error = Some(error);
            job.view.retryable = false;
            job.active_spec = None;
        }
        trim_history_locked(&mut value);
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
    use tokio::sync::Notify;

    struct CountingClient {
        info: SftpSessionInfo,
        upload_calls: AtomicUsize,
        remove_calls: AtomicUsize,
        commit_new_calls: AtomicUsize,
        commit_replace_calls: AtomicUsize,
        identity_calls: AtomicUsize,
        rename_calls: AtomicUsize,
        identities: Mutex<VecDeque<Result<Option<crate::EntryIdentity>, AppError>>>,
        upload_result: Mutex<Option<crate::StagedUploadResult>>,
        commit_new_result: Mutex<Option<Result<(), AppError>>>,
        block_upload: AtomicBool,
        upload_started: Notify,
        upload_release: Notify,
        download_bytes: Mutex<Vec<u8>>,
        competing_local_path: Mutex<Option<std::path::PathBuf>>,
        remove_local_path: Mutex<Option<std::path::PathBuf>>,
    }

    impl CountingClient {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                info: SftpSessionInfo {
                    id: SftpSessionId::new(),
                    host_id: HostId::new(),
                    host_session_id: HostSessionId::new(),
                    protocol_version: 3,
                    extensions: vec![
                        nexus_model::SftpExtension {
                            name: "hardlink@openssh.com".into(),
                            version: "1".into(),
                        },
                        nexus_model::SftpExtension {
                            name: "posix-rename@openssh.com".into(),
                            version: "1".into(),
                        },
                    ],
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
                remove_calls: AtomicUsize::new(0),
                commit_new_calls: AtomicUsize::new(0),
                commit_replace_calls: AtomicUsize::new(0),
                identity_calls: AtomicUsize::new(0),
                rename_calls: AtomicUsize::new(0),
                identities: Mutex::new(VecDeque::new()),
                upload_result: Mutex::new(None),
                commit_new_result: Mutex::new(None),
                block_upload: AtomicBool::new(false),
                upload_started: Notify::new(),
                upload_release: Notify::new(),
                download_bytes: Mutex::new(b"remote payload".to_vec()),
                competing_local_path: Mutex::new(None),
                remove_local_path: Mutex::new(None),
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
            self.identity_calls.fetch_add(1, Ordering::SeqCst);
            self.identities
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(None))
        }
        async fn create_dir(&self, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn remove_file(&self, _: &str) -> Result<(), AppError> {
            let path = self.remove_local_path.lock().unwrap().take();
            match path {
                Some(path) => std::fs::remove_file(path).map_err(|_| unused()),
                None => Err(unused()),
            }
        }
        async fn remove_dir(&self, _: &str) -> Result<(), AppError> {
            Err(unused())
        }
        async fn rename_noclobber(&self, _: &str, _: &str) -> Result<(), AppError> {
            self.rename_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn upload_staged(
            &self,
            reader: &mut (dyn AsyncRead + Unpin + Send),
            _: &str,
            ownership: StagingOwnership,
            cancel: CancellationToken,
            progress: Progress,
        ) -> crate::StagedUploadResult {
            self.upload_calls.fetch_add(1, Ordering::SeqCst);
            if self.block_upload.load(Ordering::SeqCst) {
                ownership.mark_owned();
                self.upload_started.notify_one();
                self.upload_release.notified().await;
            }
            if let Some(result) = self.upload_result.lock().unwrap().take() {
                if result.is_ok() || matches!(&result, Err(StagedUploadFailure::OwnedFailure(_))) {
                    ownership.mark_owned();
                }
                return result;
            }
            ownership.mark_owned();
            let mut bytes = 0_u64;
            let mut buffer = [0_u8; 3];
            let result: Result<(), AppError> = async {
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
                Ok(())
            }
            .await;
            if let Err(error) = result {
                return Err(StagedUploadFailure::OwnedFailure(error));
            }
            Ok(bytes)
        }
        async fn download(
            &self,
            _: &str,
            writer: &mut (dyn AsyncWrite + Unpin + Send),
            _: CancellationToken,
            progress: Progress,
        ) -> Result<u64, AppError> {
            let bytes = self.download_bytes.lock().unwrap().clone();
            tokio::io::AsyncWriteExt::write_all(writer, &bytes)
                .await
                .map_err(|_| unused())?;
            progress(bytes.len() as u64);
            if let Some(path) = self.competing_local_path.lock().unwrap().take() {
                std::fs::write(path, b"competing content").map_err(|_| unused())?;
            }
            Ok(bytes.len() as u64)
        }
        async fn commit_new(&self, _: &str, _: &str) -> Result<(), AppError> {
            self.commit_new_calls.fetch_add(1, Ordering::SeqCst);
            self.commit_new_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(()))
        }
        async fn commit_replace(&self, _: &str, _: &str) -> Result<(), AppError> {
            self.commit_replace_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn remove_owned_staging(&self, _: &str) {
            self.remove_calls.fetch_add(1, Ordering::SeqCst);
        }
        async fn close(&self) {}
    }

    fn local_item(path: &std::path::Path) -> crate::LocalItem {
        crate::open_local_source(path.canonicalize().unwrap(), "binary.dat".into()).unwrap()
    }

    #[cfg(windows)]
    async fn wait_for_state(
        manager: &TransferManager,
        id: TransferJobId,
        expected: TransferState,
    ) -> TransferJob {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(job) = manager
                    .list(None)
                    .into_iter()
                    .find(|job| job.id == id && job.state == expected)
                {
                    return job;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("transfer state watchdog")
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
            action: DestinationAction::CreateNew,
        };
        let spec = upload_spec(client.clone(), ConflictPolicy::Skip, planned).unwrap();
        run_transfer(
            &Arc::new(Mutex::new(ManagerState::default())),
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
            action: DestinationAction::Skip,
        };
        let skipped_spec = upload_spec(client.clone(), ConflictPolicy::Skip, skipped).unwrap();
        run_transfer(
            &Arc::new(Mutex::new(ManagerState::default())),
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
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(ManagerState::default())),
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
                        action: DestinationAction::CreateNew,
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

    fn test_record(
        manager: &TransferManager,
        spec: JobSpec,
        state: TransferState,
        retryable: bool,
    ) -> TransferJobId {
        let active = !terminal(&state);
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
            state,
            confirmed_bytes: "0".into(),
            total_bytes: total.map(|value| value.to_string()),
            error: None,
            retryable,
        };
        let mut manager_state = manager.state.lock().unwrap();
        manager_state.order.push_back(id);
        manager_state.jobs.insert(
            id,
            JobRecord {
                view,
                cancel: CancellationToken::new(),
                active_spec: active.then_some(spec),
            },
        );
        id
    }

    #[tokio::test]
    async fn staging_cleanup_requires_confirmed_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.bin");
        std::fs::write(&path, b"owned bytes").unwrap();
        for failure in [
            StagedUploadFailure::NotCreated(AppError::new(
                ErrorCode::Conflict,
                "Exclusive create was refused.",
            )),
            StagedUploadFailure::CreationOutcomeUnknown(AppError::new(
                ErrorCode::OutcomeUnknown,
                "Create reply was lost.",
            )),
        ] {
            let client = CountingClient::new();
            *client.upload_result.lock().unwrap() = Some(Err(failure));
            let spec = upload_spec(
                client.clone(),
                ConflictPolicy::Replace,
                PlannedUpload {
                    source: local_item(&path),
                    destination: "/tmp/preexisting.part".into(),
                    action: DestinationAction::CreateNew,
                },
            )
            .unwrap();
            assert!(
                run_transfer(
                    &Arc::new(Mutex::new(ManagerState::default())),
                    TransferJobId::new(),
                    &spec,
                    CancellationToken::new(),
                )
                .await
                .is_err()
            );
            assert_eq!(client.remove_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn owned_staging_is_cleaned_on_post_stream_failures_and_caller_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.bin");
        std::fs::write(&path, b"owned bytes").unwrap();

        let identity_failure = CountingClient::new();
        identity_failure
            .identities
            .lock()
            .unwrap()
            .push_back(Err(AppError::new(ErrorCode::Timeout, "Identity timeout.")));
        let spec = upload_spec(
            identity_failure.clone(),
            ConflictPolicy::Replace,
            PlannedUpload {
                source: local_item(&path),
                destination: "/tmp/result.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(ManagerState::default())),
                TransferJobId::new(),
                &spec,
                CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Timeout
        );
        assert_eq!(identity_failure.remove_calls.load(Ordering::SeqCst), 1);

        let cleanup_failure = CountingClient::new();
        cleanup_failure
            .identities
            .lock()
            .unwrap()
            .push_back(Ok(None));
        *cleanup_failure.commit_new_result.lock().unwrap() = Some(Err(AppError::new(
            ErrorCode::OutcomeUnknown,
            "Final exists but cleanup reply was lost.",
        )));
        let spec = upload_spec(
            cleanup_failure.clone(),
            ConflictPolicy::Replace,
            PlannedUpload {
                source: local_item(&path),
                destination: "/tmp/result.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(ManagerState::default())),
                TransferJobId::new(),
                &spec,
                CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::OutcomeUnknown
        );
        assert_eq!(cleanup_failure.remove_calls.load(Ordering::SeqCst), 1);

        let dropped = CountingClient::new();
        dropped.block_upload.store(true, Ordering::SeqCst);
        let spec = upload_spec(
            dropped.clone(),
            ConflictPolicy::KeepBoth,
            PlannedUpload {
                source: local_item(&path),
                destination: "/tmp/drop.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ManagerState::default()));
        let task = tokio::spawn(async move {
            run_transfer(
                &state,
                TransferJobId::new(),
                &spec,
                CancellationToken::new(),
            )
            .await
        });
        dropped.upload_started.notified().await;
        task.abort();
        let _ = task.await;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while dropped.remove_calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("drop cleanup watchdog");
        assert_eq!(dropped.remove_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn finalization_uses_the_per_item_approved_action() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.bin");
        std::fs::write(&path, b"new payload").unwrap();
        let client = CountingClient::new();
        client.identities.lock().unwrap().push_back(Ok(None));
        let spec = upload_spec(
            client.clone(),
            ConflictPolicy::Replace,
            PlannedUpload {
                source: local_item(&path),
                destination: "/tmp/absent-at-approval.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        run_transfer(
            &Arc::new(Mutex::new(ManagerState::default())),
            TransferJobId::new(),
            &spec,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(client.commit_new_calls.load(Ordering::SeqCst), 1);
        assert_eq!(client.commit_replace_calls.load(Ordering::SeqCst), 0);

        let competitor = CountingClient::new();
        competitor
            .identities
            .lock()
            .unwrap()
            .push_back(Ok(Some(crate::EntryIdentity {
                kind: RemoteEntryKind::File,
                size: Some(19),
                modified: Some(7),
            })));
        let spec = upload_spec(
            competitor.clone(),
            ConflictPolicy::Replace,
            PlannedUpload {
                source: local_item(&path),
                destination: "/tmp/appeared.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(ManagerState::default())),
                TransferJobId::new(),
                &spec,
                CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
        assert_eq!(competitor.commit_new_calls.load(Ordering::SeqCst), 0);
        assert_eq!(competitor.commit_replace_calls.load(Ordering::SeqCst), 0);
        assert_eq!(competitor.remove_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn download_replace_preference_cannot_overwrite_a_new_target() {
        let directory = tempfile::tempdir().unwrap();
        let selected =
            crate::open_local_directory(directory.path().canonicalize().unwrap()).unwrap();
        let destination = directory.path().join("download.bin");
        let source_identity = crate::EntryIdentity {
            kind: RemoteEntryKind::File,
            size: Some(14),
            modified: Some(11),
        };
        let client = CountingClient::new();
        client
            .download_bytes
            .lock()
            .unwrap()
            .clone_from(&b"remote payload".to_vec());
        client.identities.lock().unwrap().extend([
            Ok(Some(source_identity.clone())),
            Ok(Some(source_identity.clone())),
        ]);
        *client.competing_local_path.lock().unwrap() = Some(destination.clone());
        let spec = download_spec(
            client,
            ConflictPolicy::Replace,
            PlannedDownload {
                source: crate::RemoteSource {
                    path: "/tmp/download.bin".into(),
                    name: "download.bin".into(),
                    size: Some(14),
                    identity: source_identity,
                },
                destination: selected,
                file_name: "download.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(ManagerState::default())),
                TransferJobId::new(),
                &spec,
                CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
        assert_eq!(std::fs::read(destination).unwrap(), b"competing content");
    }

    #[tokio::test]
    async fn changed_remote_source_blocks_download_finalization() {
        let directory = tempfile::tempdir().unwrap();
        let selected =
            crate::open_local_directory(directory.path().canonicalize().unwrap()).unwrap();
        let approved = crate::EntryIdentity {
            kind: RemoteEntryKind::File,
            size: Some(14),
            modified: Some(11),
        };
        let changed = crate::EntryIdentity {
            modified: Some(12),
            ..approved.clone()
        };
        let client = CountingClient::new();
        client
            .identities
            .lock()
            .unwrap()
            .extend([Ok(Some(approved.clone())), Ok(Some(changed))]);
        let spec = download_spec(
            client,
            ConflictPolicy::Skip,
            PlannedDownload {
                source: crate::RemoteSource {
                    path: "/tmp/source.bin".into(),
                    name: "source.bin".into(),
                    size: Some(14),
                    identity: approved,
                },
                destination: selected,
                file_name: "source.bin".into(),
                action: DestinationAction::CreateNew,
            },
        )
        .unwrap();
        assert_eq!(
            run_transfer(
                &Arc::new(Mutex::new(ManagerState::default())),
                TransferJobId::new(),
                &spec,
                CancellationToken::new(),
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
        assert!(!directory.path().join("source.bin").exists());
    }

    #[tokio::test]
    async fn rename_requires_the_approved_source_identity() {
        let client = CountingClient::new();
        let approved = crate::EntryIdentity {
            kind: RemoteEntryKind::File,
            size: Some(8),
            modified: Some(1),
        };
        client
            .identities
            .lock()
            .unwrap()
            .push_back(Ok(Some(crate::EntryIdentity {
                modified: Some(2),
                ..approved.clone()
            })));
        assert_eq!(
            execute_mutation(
                client.clone(),
                crate::MutationSpec::Rename {
                    source: "/tmp/old.bin".into(),
                    destination: "/tmp/new.bin".into(),
                    expected_source: approved,
                },
            )
            .await
            .unwrap_err()
            .code,
            ErrorCode::Conflict
        );
        assert_eq!(client.rename_calls.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_delete_removes_the_link_and_preserves_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.bin");
        let link = directory.path().join("link.bin");
        std::fs::write(&target, b"distinguishable target").unwrap();
        symlink(&target, &link).unwrap();
        let expected = crate::EntryIdentity {
            kind: RemoteEntryKind::Symlink,
            size: Some(7),
            modified: Some(1),
        };
        let client = CountingClient::new();
        client
            .identities
            .lock()
            .unwrap()
            .push_back(Ok(Some(expected.clone())));
        *client.remove_local_path.lock().unwrap() = Some(link.clone());
        execute_mutation(
            client,
            crate::MutationSpec::Delete {
                path: "/tmp/link.bin".into(),
                kind: RemoteEntryKind::Symlink,
                expected,
            },
        )
        .await
        .unwrap();
        assert!(!link.exists());
        assert_eq!(std::fs::read(target).unwrap(), b"distinguishable target");
    }

    #[cfg(windows)]
    #[test]
    fn selected_upload_handle_blocks_path_replacement_and_writes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("locked.bin");
        std::fs::write(&path, b"approved").unwrap();
        let selected = local_item(&path);
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::rename(&path, directory.path().join("moved.bin")).is_err());
        validate_local_source(&selected).unwrap();
    }

    #[tokio::test]
    async fn retry_requires_new_explicit_local_selection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retry.bin");
        std::fs::write(&path, b"retry payload").unwrap();
        let client = CountingClient::new();
        let make_spec = || {
            upload_spec(
                client.clone(),
                ConflictPolicy::Replace,
                PlannedUpload {
                    source: local_item(&path),
                    destination: "/tmp/retry.bin".into(),
                    action: DestinationAction::CreateNew,
                },
            )
            .unwrap()
        };
        let manager = TransferManager::new();
        let unknown = test_record(&manager, make_spec(), TransferState::OutcomeUnknown, false);
        let permanent = test_record(&manager, make_spec(), TransferState::Failed, false);
        let retryable = test_record(&manager, make_spec(), TransferState::Failed, true);
        let info = client.info();
        let store = FilePlanStore::default();
        assert_eq!(
            manager
                .retry_plan(&store, unknown, info.host_id, info.host_session_id, info.id)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Policy
        );
        assert_eq!(
            manager
                .retry_plan(
                    &store,
                    permanent,
                    info.host_id,
                    info.host_session_id,
                    info.id
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::Policy
        );
        assert_eq!(client.identity_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            manager
                .retry_plan(
                    &store,
                    retryable,
                    info.host_id,
                    info.host_session_id,
                    info.id,
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::Policy
        );
        assert_eq!(client.identity_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn batch_admission_is_atomic_at_capacity_and_under_parallel_submit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.bin");
        std::fs::write(&path, b"queued").unwrap();
        let client = CountingClient::new();
        let manager = Arc::new(TransferManager::new());
        let held_global = manager
            .global
            .clone()
            .acquire_many_owned(GLOBAL_ACTIVE as u32)
            .await
            .unwrap();
        let spec_for = |number: usize| {
            upload_spec(
                client.clone(),
                ConflictPolicy::KeepBoth,
                PlannedUpload {
                    source: local_item(&path),
                    destination: format!("/tmp/queued-{number}.bin"),
                    action: DestinationAction::CreateNew,
                },
            )
            .unwrap()
        };
        let initial = (0..99).map(&spec_for).collect();
        assert_eq!(manager.enqueue_batch(initial).await.unwrap().len(), 99);
        let before = manager.list(None).len();
        let left = manager.clone();
        let right = manager.clone();
        let left_specs = vec![spec_for(100), spec_for(101)];
        let right_specs = vec![spec_for(102), spec_for(103)];
        let (left_result, right_result) = tokio::join!(
            left.enqueue_batch(left_specs),
            right.enqueue_batch(right_specs)
        );
        assert!(left_result.is_err());
        assert!(right_result.is_err());
        assert_eq!(manager.list(None).len(), before);

        let first = manager.list(None)[0].clone();
        manager
            .cancel(
                first.id,
                first.host_id,
                first.host_session_id,
                first.sftp_session_id,
            )
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if manager
                    .list(None)
                    .iter()
                    .any(|job| job.id == first.id && job.state == TransferState::Cancelled)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled slot release watchdog");
        assert_eq!(
            manager
                .enqueue_batch(vec![spec_for(104)])
                .await
                .unwrap()
                .len(),
            1
        );
        manager.shutdown();
        drop(held_global);
    }

    #[tokio::test]
    async fn listing_and_state_updates_share_one_lock_with_a_watchdog() {
        let manager = Arc::new(TransferManager::new());
        let held = manager.state.lock().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let thread_manager = manager.clone();
        let thread_barrier = barrier.clone();
        let listing = std::thread::spawn(move || {
            thread_barrier.wait();
            thread_manager.list(None)
        });
        barrier.wait();
        drop(held);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::task::spawn_blocking(move || listing.join().unwrap()),
        )
        .await
        .expect("single-lock listing watchdog")
        .unwrap();
    }

    #[tokio::test]
    async fn jobs_for_one_destination_are_serialized() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("same.bin");
        std::fs::write(&path, b"same destination").unwrap();
        let client = CountingClient::new();
        client.block_upload.store(true, Ordering::SeqCst);
        let manager = TransferManager::new();
        let specs = (0..2)
            .map(|_| {
                upload_spec(
                    client.clone(),
                    ConflictPolicy::KeepBoth,
                    PlannedUpload {
                        source: local_item(&path),
                        destination: "/tmp/same.bin".into(),
                        action: DestinationAction::CreateNew,
                    },
                )
                .unwrap()
            })
            .collect();
        manager.enqueue_batch(specs).await.unwrap();
        client.upload_started.notified().await;
        tokio::task::yield_now().await;
        assert_eq!(client.upload_calls.load(Ordering::SeqCst), 1);
        client.block_upload.store(false, Ordering::SeqCst);
        client.upload_release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if manager
                    .list(None)
                    .iter()
                    .all(|job| job.state == TransferState::Completed)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("destination serialization watchdog");
        assert_eq!(client.upload_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn completed_history_is_strictly_bounded() {
        let client = CountingClient::new();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.bin");
        std::fs::write(&path, b"history").unwrap();
        let manager = TransferManager::new();
        for index in 0..=HISTORY_CAP {
            let spec = upload_spec(
                client.clone(),
                ConflictPolicy::KeepBoth,
                PlannedUpload {
                    source: local_item(&path),
                    destination: format!("/tmp/history-{index}.bin"),
                    action: DestinationAction::CreateNew,
                },
            )
            .unwrap();
            test_record(&manager, spec, TransferState::Completed, false);
        }
        trim_history_locked(&mut manager.state.lock().unwrap());
        assert_eq!(manager.list(None).len(), HISTORY_CAP);
    }

    #[test]
    fn destination_keys_use_object_identity_and_final_name() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let first_directory =
            crate::open_local_directory(first.path().canonicalize().unwrap()).unwrap();
        let alias = first_directory.clone();
        let second_directory =
            crate::open_local_directory(second.path().canonicalize().unwrap()).unwrap();
        let client = CountingClient::new();
        let make_spec = |directory| {
            download_spec(
                client.clone(),
                ConflictPolicy::KeepBoth,
                PlannedDownload {
                    source: crate::RemoteSource {
                        path: "/tmp/source.bin".into(),
                        name: "source.bin".into(),
                        size: Some(1),
                        identity: crate::EntryIdentity {
                            kind: RemoteEntryKind::File,
                            size: Some(1),
                            modified: Some(1),
                        },
                    },
                    destination: directory,
                    file_name: "source.bin".into(),
                    action: DestinationAction::CreateNew,
                },
            )
            .unwrap()
        };
        let first_key = destination_key(&make_spec(first_directory));
        let alias_key = destination_key(&make_spec(alias));
        let second_key = destination_key(&make_spec(second_directory));
        assert_eq!(first_key, alias_key);
        assert_ne!(first_key, second_key);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn terminal_upload_history_releases_local_handles() {
        let parent = tempfile::tempdir().unwrap();

        let completed_path = parent.path().join("completed.bin");
        std::fs::write(&completed_path, b"completed").unwrap();
        let completed_client = CountingClient::new();
        let completed_manager = TransferManager::new();
        let completed = completed_manager
            .enqueue(
                upload_spec(
                    completed_client,
                    ConflictPolicy::KeepBoth,
                    PlannedUpload {
                        source: local_item(&completed_path),
                        destination: "/tmp/completed.bin".into(),
                        action: DestinationAction::CreateNew,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        wait_for_state(&completed_manager, completed.id, TransferState::Completed).await;
        assert_eq!(completed_manager.list(None).len(), 1);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&completed_path)
            .unwrap();
        std::fs::rename(
            &completed_path,
            parent.path().join("completed-released.bin"),
        )
        .unwrap();

        for (name, failure, expected) in [
            (
                "failed",
                StagedUploadFailure::NotCreated(AppError::new(
                    ErrorCode::Conflict,
                    "Permanent failure.",
                )),
                TransferState::Failed,
            ),
            (
                "unknown",
                StagedUploadFailure::CreationOutcomeUnknown(AppError::new(
                    ErrorCode::OutcomeUnknown,
                    "Unknown outcome.",
                )),
                TransferState::OutcomeUnknown,
            ),
        ] {
            let path = parent.path().join(format!("{name}.bin"));
            std::fs::write(&path, name.as_bytes()).unwrap();
            let client = CountingClient::new();
            *client.upload_result.lock().unwrap() = Some(Err(failure));
            let manager = TransferManager::new();
            let job = manager
                .enqueue(
                    upload_spec(
                        client,
                        ConflictPolicy::KeepBoth,
                        PlannedUpload {
                            source: local_item(&path),
                            destination: format!("/tmp/{name}.bin"),
                            action: DestinationAction::CreateNew,
                        },
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
            wait_for_state(&manager, job.id, expected).await;
            assert_eq!(manager.list(None).len(), 1);
            std::fs::rename(&path, parent.path().join(format!("{name}-released.bin"))).unwrap();
        }

        let cancelled_path = parent.path().join("cancelled.bin");
        std::fs::write(&cancelled_path, b"cancelled").unwrap();
        let cancelled_client = CountingClient::new();
        let cancelled_manager = TransferManager::new();
        let held = cancelled_manager
            .global
            .clone()
            .acquire_many_owned(GLOBAL_ACTIVE as u32)
            .await
            .unwrap();
        let cancelled = cancelled_manager
            .enqueue(
                upload_spec(
                    cancelled_client,
                    ConflictPolicy::KeepBoth,
                    PlannedUpload {
                        source: local_item(&cancelled_path),
                        destination: "/tmp/cancelled.bin".into(),
                        action: DestinationAction::CreateNew,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        cancelled_manager
            .cancel(
                cancelled.id,
                cancelled.host_id,
                cancelled.host_session_id,
                cancelled.sftp_session_id,
            )
            .unwrap();
        wait_for_state(&cancelled_manager, cancelled.id, TransferState::Cancelled).await;
        std::fs::rename(
            &cancelled_path,
            parent.path().join("cancelled-released.bin"),
        )
        .unwrap();
        drop(held);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn completed_download_history_releases_destination_directory() {
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("destination");
        std::fs::create_dir(&destination).unwrap();
        let client = CountingClient::new();
        let identity = crate::EntryIdentity {
            kind: RemoteEntryKind::File,
            size: Some(14),
            modified: Some(1),
        };
        client
            .identities
            .lock()
            .unwrap()
            .extend([Ok(Some(identity.clone())), Ok(Some(identity.clone()))]);
        let manager = TransferManager::new();
        let job = manager
            .enqueue(
                download_spec(
                    client,
                    ConflictPolicy::KeepBoth,
                    PlannedDownload {
                        source: crate::RemoteSource {
                            path: "/tmp/source.bin".into(),
                            name: "source.bin".into(),
                            size: Some(14),
                            identity,
                        },
                        destination: crate::open_local_directory(
                            destination.canonicalize().unwrap(),
                        )
                        .unwrap(),
                        file_name: "source.bin".into(),
                        action: DestinationAction::CreateNew,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        wait_for_state(&manager, job.id, TransferState::Completed).await;
        assert_eq!(manager.list(None).len(), 1);
        std::fs::rename(&destination, parent.path().join("destination-released")).unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancelling_one_job_does_not_release_another_active_lease() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("shared.bin");
        std::fs::write(&path, b"shared").unwrap();
        let client = CountingClient::new();
        let manager = TransferManager::new();
        let held = manager
            .global
            .clone()
            .acquire_many_owned(GLOBAL_ACTIVE as u32)
            .await
            .unwrap();
        let make_spec = |suffix: &str| {
            upload_spec(
                client.clone(),
                ConflictPolicy::KeepBoth,
                PlannedUpload {
                    source: local_item(&path),
                    destination: format!("/tmp/{suffix}.bin"),
                    action: DestinationAction::CreateNew,
                },
            )
            .unwrap()
        };
        let jobs = manager
            .enqueue_batch(vec![make_spec("first"), make_spec("second")])
            .await
            .unwrap();
        manager
            .cancel(
                jobs[0].id,
                jobs[0].host_id,
                jobs[0].host_session_id,
                jobs[0].sftp_session_id,
            )
            .unwrap();
        wait_for_state(&manager, jobs[0].id, TransferState::Cancelled).await;
        assert!(std::fs::rename(&path, parent.path().join("still-leased.bin")).is_err());
        manager
            .cancel(
                jobs[1].id,
                jobs[1].host_id,
                jobs[1].host_session_id,
                jobs[1].sftp_session_id,
            )
            .unwrap();
        wait_for_state(&manager, jobs[1].id, TransferState::Cancelled).await;
        std::fs::rename(&path, parent.path().join("all-released.bin")).unwrap();
        drop(held);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn active_disconnect_cancels_once_cleans_owned_staging_and_releases_source() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("disconnect.bin");
        std::fs::write(&path, b"disconnect").unwrap();
        let client = CountingClient::new();
        client.block_upload.store(true, Ordering::SeqCst);
        let info = client.info();
        let manager = TransferManager::new();
        let job = manager
            .enqueue(
                upload_spec(
                    client.clone(),
                    ConflictPolicy::KeepBoth,
                    PlannedUpload {
                        source: local_item(&path),
                        destination: "/tmp/disconnect.bin".into(),
                        action: DestinationAction::CreateNew,
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        client.upload_started.notified().await;
        manager.disconnect(info.host_id, info.host_session_id);
        client.upload_release.notify_one();
        wait_for_state(&manager, job.id, TransferState::Cancelled).await;
        assert_eq!(client.upload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(client.remove_calls.load(Ordering::SeqCst), 1);
        std::fs::rename(&path, parent.path().join("disconnect-released.bin")).unwrap();
    }
}
