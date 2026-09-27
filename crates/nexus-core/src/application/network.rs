use super::*;

fn stale_session() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The network request does not belong to the active host session.",
    )
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "A network inventory request is already in progress or the network limit is reached.",
    )
}

impl Application {
    /// A session-bound, non-queuing observation. No network data is retained.
    pub async fn list_host_network(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<NetworkSnapshot, AppError> {
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
            let mut gates = self.network_gates.lock().await;
            gates
                .entry(host_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _host_permit = gate.try_lock().map_err(|_| busy())?;
        let _global_permit = self.network_limit.try_acquire().map_err(|_| busy())?;
        drop(mutation);
        let result = nexus_discovery::observe_network(
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
