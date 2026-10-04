use nexus_model::{AppError, ErrorCode, HostId};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

pub struct ExecutionAdmission {
    host_gates: std::sync::Mutex<HashMap<HostId, Arc<AsyncMutex<()>>>>,
    global: Arc<Semaphore>,
}

impl Default for ExecutionAdmission {
    fn default() -> Self {
        Self {
            host_gates: std::sync::Mutex::new(HashMap::new()),
            global: Arc::new(Semaphore::new(1)),
        }
    }
}

pub struct ExecutionPermit {
    pub host_id: HostId,
    _host: OwnedMutexGuard<()>,
    _global: OwnedSemaphorePermit,
}

impl ExecutionAdmission {
    /// Execution is never queued. Failure leaves plan authority pending.
    pub fn try_execute(&self, host_id: HostId) -> Result<ExecutionPermit, AppError> {
        let host = self.host_gate(host_id)?;
        let host = host.try_lock_owned().map_err(|_| busy())?;
        let global = Arc::clone(&self.global)
            .try_acquire_owned()
            .map_err(|_| busy())?;
        Ok(ExecutionPermit {
            host_id,
            _host: host,
            _global: global,
        })
    }

    /// Host lifecycle work may wait, but callers must acquire this guard before
    /// the global Application metadata mutex so unrelated hosts stay unblocked.
    pub async fn lifecycle_guard(&self, host_id: HostId) -> Result<OwnedMutexGuard<()>, AppError> {
        Ok(self.host_gate(host_id)?.lock_owned().await)
    }

    /// User-facing publication never waits behind lifecycle or execution work.
    pub fn try_lifecycle_guard(&self, host_id: HostId) -> Result<OwnedMutexGuard<()>, AppError> {
        self.host_gate(host_id)?
            .try_lock_owned()
            .map_err(|_| busy())
    }

    fn host_gate(&self, host_id: HostId) -> Result<Arc<AsyncMutex<()>>, AppError> {
        let mut gates = self.host_gates.lock().map_err(|_| unavailable())?;
        Ok(gates
            .entry(host_id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone())
    }
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Another remote operation is already in progress.",
    )
}

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Remote operation admission is unavailable.",
    )
}
