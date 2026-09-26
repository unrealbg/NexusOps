use super::*;

fn stale_session() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The service request does not belong to the active host session.",
    )
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "A service inventory request is already in progress or the service limit is reached.",
    )
}

impl Application {
    /// A session-bound, non-queuing inventory request. Only admission primitives
    /// are retained in the application; service data is returned to the caller.
    pub async fn list_host_services(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<ServiceSnapshot, AppError> {
        // Serialize admission with delete/shutdown only until ownership is
        // captured. Neither metadata gate is held over remote I/O.
        let mutation = self.mutation.try_lock().map_err(|_| busy())?;
        self.repository.get(host_id)?;
        let slot = self.slot(host_id).await;
        let (transport, cancellation, generation) = {
            let data = slot.data.lock().await;
            if data.view.state != ConnectionState::Connected
                || data.connection_id != Some(host_session_id)
                || data.view.host_session_id != Some(host_session_id)
            {
                return Err(stale_session());
            }
            (
                data.transport.clone().ok_or_else(stale_session)?,
                data.cancel.clone(),
                data.generation,
            )
        };
        let gate = {
            let mut gates = self.service_gates.lock().await;
            gates
                .entry(host_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _host_permit = gate.try_lock().map_err(|_| busy())?;
        let _global_permit = self.service_limit.try_acquire().map_err(|_| busy())?;
        drop(mutation);
        let result = nexus_discovery::observe_services(
            transport.as_ref(),
            cancellation.clone(),
            host_id,
            host_session_id,
        )
        .await;
        let data = slot.data.lock().await;
        if cancellation.is_cancelled()
            || data.generation != generation
            || data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
            || data.view.host_session_id != Some(host_session_id)
        {
            return Err(cancelled());
        }
        result
    }
}
