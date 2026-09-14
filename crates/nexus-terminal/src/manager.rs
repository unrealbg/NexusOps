use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, TerminalOutputBatch, TerminalSession,
    TerminalSessionId, TerminalSize, TerminalState,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub const MAX_OPEN_TERMINALS_PER_HOST: usize = 8;
pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
pub const OUTPUT_QUEUE_CAPACITY: usize = 64;
pub const OUTPUT_BATCH_BYTES: usize = 64 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const TOMBSTONE_LIMIT: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum TerminalRead {
    Data(Vec<u8>),
    Exit(u32),
    Closed,
}

#[async_trait]
pub trait TerminalChannel: Send + Sync {
    async fn read(&self) -> Result<TerminalRead, AppError>;
    async fn write(&self, data: &[u8]) -> Result<(), AppError>;
    async fn resize(&self, size: TerminalSize) -> Result<(), AppError>;
    async fn close(&self) -> Result<(), AppError>;
}

#[async_trait]
pub trait TerminalConnector: Send + Sync {
    async fn open_terminal(
        &self,
        size: TerminalSize,
        cancellation: CancellationToken,
    ) -> Result<Arc<dyn TerminalChannel>, AppError>;
}

struct Entry {
    view: RwLock<TerminalSession>,
    channel: Mutex<Option<Arc<dyn TerminalChannel>>>,
    output: Mutex<mpsc::Receiver<Vec<u8>>>,
    cancel: CancellationToken,
    resize_gate: Mutex<()>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Ownership {
    host_id: HostId,
    host_session_id: HostSessionId,
    terminal_id: TerminalSessionId,
}

#[derive(Default)]
struct ManagerState {
    sessions: HashMap<TerminalSessionId, Arc<Entry>>,
    next_label: HashMap<HostId, u32>,
    closed: VecDeque<Ownership>,
}

#[derive(Default)]
pub struct TerminalManager {
    inner: Mutex<ManagerState>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn open<C: TerminalConnector + ?Sized>(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        connector: &C,
        size: TerminalSize,
    ) -> Result<TerminalSession, AppError> {
        let size = size.validate()?;
        let (entry, output_tx) = {
            let mut manager = self.inner.lock().await;
            let open_count = manager
                .sessions
                .values()
                .filter(|entry| {
                    let view = entry.view.read().expect("terminal view poisoned");
                    view.host_id == host_id
                        && !matches!(
                            view.state,
                            TerminalState::Closed
                                | TerminalState::Failed
                                | TerminalState::Disconnected
                        )
                })
                .count();
            if open_count >= MAX_OPEN_TERMINALS_PER_HOST {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This host already has the maximum of eight terminal sessions.",
                ));
            }
            let number = manager.next_label.entry(host_id).or_insert(0);
            *number = number.saturating_add(1);
            let view = TerminalSession {
                id: TerminalSessionId::new(),
                host_id,
                host_session_id,
                label: format!("Terminal {number}"),
                state: TerminalState::Creating,
                size,
                error: None,
            };
            let (output_tx, output_rx) = mpsc::channel(OUTPUT_QUEUE_CAPACITY);
            let entry = Arc::new(Entry {
                view: RwLock::new(view.clone()),
                channel: Mutex::new(None),
                output: Mutex::new(output_rx),
                cancel: CancellationToken::new(),
                resize_gate: Mutex::new(()),
            });
            manager.sessions.insert(view.id, entry.clone());
            (entry, output_tx)
        };

        let open_result = tokio::select! {
            biased;
            _ = entry.cancel.cancelled() => Err(cancelled()),
            result = tokio::time::timeout(
                STARTUP_TIMEOUT,
                connector.open_terminal(size, entry.cancel.child_token()),
            ) => result.map_err(|_| AppError::new(
                ErrorCode::TerminalStartup,
                "The remote terminal did not start within ten seconds.",
            ))?,
        };

        let channel = match open_result {
            Ok(channel) if !entry.cancel.is_cancelled() => channel,
            Ok(channel) => {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, channel.close()).await;
                self.fail_entry(&entry, cancelled(), TerminalState::Disconnected);
                return Err(cancelled());
            }
            Err(error) => {
                let state = if error.code == ErrorCode::Cancelled {
                    TerminalState::Disconnected
                } else {
                    TerminalState::Failed
                };
                self.fail_entry(&entry, error.clone(), state);
                return Err(error);
            }
        };

        let accepted = {
            let mut channel_slot = entry.channel.lock().await;
            let mut view = entry.view.write().expect("terminal view poisoned");
            if entry.cancel.is_cancelled() || view.state != TerminalState::Creating {
                false
            } else {
                *channel_slot = Some(channel.clone());
                view.state = TerminalState::Open;
                true
            }
        };
        if !accepted {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, channel.close()).await;
            return Err(cancelled());
        }
        let output_entry = entry.clone();
        tokio::spawn(async move {
            pump_output(output_entry, channel, output_tx).await;
        });
        tracing::info!(
            host_id = %host_id,
            terminal_id = %entry.snapshot().id,
            "terminal session created"
        );
        Ok(entry.snapshot())
    }

    pub async fn list(&self, host_id: HostId) -> Vec<TerminalSession> {
        let manager = self.inner.lock().await;
        let mut sessions: Vec<_> = manager
            .sessions
            .values()
            .map(|entry| entry.snapshot())
            .filter(|view| view.host_id == host_id)
            .collect();
        sessions.sort_by(|left, right| left.label.cmp(&right.label));
        sessions
    }

    pub async fn poll(
        &self,
        ownership: (HostId, HostSessionId, TerminalSessionId),
    ) -> Result<TerminalOutputBatch, AppError> {
        let entry = self.entry(ownership).await?;
        let mut receiver = entry.output.lock().await;
        let mut chunks = Vec::new();
        let mut total = 0usize;
        while total < OUTPUT_BATCH_BYTES {
            match receiver.try_recv() {
                Ok(chunk) => {
                    total += chunk.len();
                    chunks.push(STANDARD.encode(chunk));
                }
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
        Ok(TerminalOutputBatch {
            session: entry.snapshot(),
            chunks_base64: chunks,
        })
    }

    pub async fn write(
        &self,
        ownership: (HostId, HostSessionId, TerminalSessionId),
        data_base64: &str,
    ) -> Result<(), AppError> {
        if data_base64.len() > (MAX_INPUT_BYTES * 4 / 3) + 8 {
            return Err(invalid_input());
        }
        let data = STANDARD.decode(data_base64).map_err(|_| invalid_input())?;
        if data.is_empty() || data.len() > MAX_INPUT_BYTES {
            return Err(invalid_input());
        }
        let entry = self.entry(ownership).await?;
        self.require_open(&entry)?;
        let channel = entry.channel.lock().await.clone().ok_or_else(unavailable)?;
        tokio::time::timeout(IO_TIMEOUT, channel.write(&data))
            .await
            .map_err(|_| AppError::new(ErrorCode::TerminalStream, "Terminal input timed out."))??;
        Ok(())
    }

    pub async fn resize(
        &self,
        ownership: (HostId, HostSessionId, TerminalSessionId),
        size: TerminalSize,
    ) -> Result<(), AppError> {
        let size = size.validate()?;
        let entry = self.entry(ownership).await?;
        self.require_open(&entry)?;
        let _gate = entry.resize_gate.lock().await;
        let channel = entry.channel.lock().await.clone().ok_or_else(unavailable)?;
        tokio::time::timeout(IO_TIMEOUT, channel.resize(size))
            .await
            .map_err(|_| {
                AppError::new(ErrorCode::TerminalResize, "Terminal resize timed out.")
            })??;
        entry.view.write().expect("terminal view poisoned").size = size;
        Ok(())
    }

    pub async fn rename(
        &self,
        ownership: (HostId, HostSessionId, TerminalSessionId),
        label: &str,
    ) -> Result<TerminalSession, AppError> {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 64 || label.chars().any(char::is_control) {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Terminal names must contain 1 to 64 printable characters.",
            ));
        }
        let entry = self.entry(ownership).await?;
        entry.view.write().expect("terminal view poisoned").label = label.to_owned();
        Ok(entry.snapshot())
    }

    pub async fn close(
        &self,
        ownership: (HostId, HostSessionId, TerminalSessionId),
    ) -> Result<(), AppError> {
        let key = Ownership {
            host_id: ownership.0,
            host_session_id: ownership.1,
            terminal_id: ownership.2,
        };
        let entry = {
            let manager = self.inner.lock().await;
            if manager.closed.contains(&key) {
                return Ok(());
            }
            manager.sessions.get(&key.terminal_id).cloned()
        }
        .ok_or_else(not_found)?;
        verify_ownership(&entry.snapshot(), key)?;
        close_entry(&entry, TerminalState::Closed).await;
        let mut manager = self.inner.lock().await;
        manager.sessions.remove(&key.terminal_id);
        manager.closed.push_back(key);
        if manager.closed.len() > TOMBSTONE_LIMIT {
            manager.closed.pop_front();
        }
        tracing::info!(host_id=%key.host_id,terminal_id=%key.terminal_id,"terminal session closed");
        Ok(())
    }

    pub async fn disconnect_connection(&self, host_id: HostId, host_session_id: HostSessionId) {
        let entries: Vec<_> = self
            .inner
            .lock()
            .await
            .sessions
            .values()
            .filter(|entry| {
                let view = entry.snapshot();
                view.host_id == host_id && view.host_session_id == host_session_id
            })
            .cloned()
            .collect();
        for entry in entries {
            close_entry(&entry, TerminalState::Disconnected).await;
        }
    }

    pub async fn remove_host(&self, host_id: HostId) {
        let entries: Vec<_> = self
            .inner
            .lock()
            .await
            .sessions
            .values()
            .filter(|entry| entry.snapshot().host_id == host_id)
            .cloned()
            .collect();
        for entry in &entries {
            close_entry(entry, TerminalState::Closed).await;
        }
        let mut manager = self.inner.lock().await;
        manager
            .sessions
            .retain(|_, entry| entry.snapshot().host_id != host_id);
        manager.closed.retain(|key| key.host_id != host_id);
        manager.next_label.remove(&host_id);
    }

    pub async fn shutdown(&self) {
        let entries: Vec<_> = self.inner.lock().await.sessions.values().cloned().collect();
        for entry in entries {
            close_entry(&entry, TerminalState::Closed).await;
        }
        let mut manager = self.inner.lock().await;
        manager.sessions.clear();
        manager.closed.clear();
    }

    async fn entry(
        &self,
        ownership: (HostId, HostSessionId, TerminalSessionId),
    ) -> Result<Arc<Entry>, AppError> {
        let key = Ownership {
            host_id: ownership.0,
            host_session_id: ownership.1,
            terminal_id: ownership.2,
        };
        let entry = self
            .inner
            .lock()
            .await
            .sessions
            .get(&key.terminal_id)
            .cloned()
            .ok_or_else(not_found)?;
        verify_ownership(&entry.snapshot(), key)?;
        Ok(entry)
    }

    fn require_open(&self, entry: &Entry) -> Result<(), AppError> {
        if entry.snapshot().state != TerminalState::Open || entry.cancel.is_cancelled() {
            return Err(unavailable());
        }
        Ok(())
    }

    fn fail_entry(&self, entry: &Entry, error: AppError, state: TerminalState) {
        let mut view = entry.view.write().expect("terminal view poisoned");
        view.state = state;
        view.error = Some(error);
    }
}

impl Entry {
    fn snapshot(&self) -> TerminalSession {
        self.view.read().expect("terminal view poisoned").clone()
    }
}

async fn pump_output(
    entry: Arc<Entry>,
    channel: Arc<dyn TerminalChannel>,
    output: mpsc::Sender<Vec<u8>>,
) {
    loop {
        let event = tokio::select! {
            biased;
            _ = entry.cancel.cancelled() => break,
            result = channel.read() => result,
        };
        match event {
            Ok(TerminalRead::Data(data)) => {
                for chunk in data.chunks(OUTPUT_CHUNK_BYTES) {
                    let send = output.send(chunk.to_vec());
                    if tokio::select! {
                        biased;
                        _ = entry.cancel.cancelled() => false,
                        result = send => result.is_ok(),
                    } {
                        continue;
                    }
                    return;
                }
            }
            Ok(TerminalRead::Exit(_)) | Ok(TerminalRead::Closed) => {
                let mut view = entry.view.write().expect("terminal view poisoned");
                if view.state == TerminalState::Open {
                    view.state = TerminalState::Closed;
                }
                break;
            }
            Err(error) => {
                let mut view = entry.view.write().expect("terminal view poisoned");
                if view.state == TerminalState::Open {
                    view.state = if error.code == ErrorCode::Connection {
                        TerminalState::Disconnected
                    } else {
                        TerminalState::Failed
                    };
                    view.error = Some(error);
                }
                break;
            }
        }
    }

    if let Some(channel) = entry.channel.lock().await.take() {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, channel.close()).await;
    }
}

async fn close_entry(entry: &Entry, final_state: TerminalState) {
    {
        let mut view = entry.view.write().expect("terminal view poisoned");
        if view.state == TerminalState::Disconnected && final_state != TerminalState::Closed {
            return;
        }
        view.state = TerminalState::Closing;
    }
    entry.cancel.cancel();
    if let Some(channel) = entry.channel.lock().await.take() {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, channel.close()).await;
    }
    while entry.output.lock().await.try_recv().is_ok() {}
    entry.view.write().expect("terminal view poisoned").state = final_state;
}

fn verify_ownership(view: &TerminalSession, key: Ownership) -> Result<(), AppError> {
    if view.host_id != key.host_id || view.host_session_id != key.host_session_id {
        return Err(AppError::new(
            ErrorCode::Policy,
            "This terminal does not belong to the supplied host connection.",
        ));
    }
    Ok(())
}

fn invalid_input() -> AppError {
    AppError::new(
        ErrorCode::Validation,
        "Terminal input must be non-empty base64 data no larger than 16 KiB.",
    )
}

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::TerminalUnavailable,
        "This terminal is no longer open. Open a new terminal to continue.",
    )
}

fn not_found() -> AppError {
    AppError::new(ErrorCode::NotFound, "The terminal session was not found.")
}

fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Terminal creation was cancelled.")
}

#[cfg(test)]
mod tests;
