use super::*;

impl Application {
    pub async fn get_session(&self, id: HostId) -> Result<HostSession, AppError> {
        self.repository.get(id)?;
        let slot = self.slot(id).await;
        let mut data = slot.data.lock().await;
        let closed_connection = if data.transport.as_ref().is_some_and(|t| t.is_closed())
            && data.view.state == ConnectionState::Connected
        {
            data.generation += 1;
            data.refreshing = false;
            data.cancel.cancel();
            data.transport = None;
            let connection_id = data.connection_id.take();
            data.view.state = data.view.state.transition(ConnectionState::Failed)?;
            data.view.error = Some(AppError::new(
                ErrorCode::Connection,
                "The remote host closed the SSH connection. Reconnect to continue.",
            ));
            data.view.discovery = None;
            data.view.capabilities.clear();
            connection_id
        } else {
            None
        };
        let view = data.view.clone();
        drop(data);
        if let Some(connection_id) = closed_connection {
            self.close_sftp(id, connection_id).await;
            self.terminals
                .disconnect_connection(id, connection_id)
                .await;
        }
        Ok(view)
    }
    pub async fn disconnect_host(&self, id: HostId) -> Result<(), AppError> {
        let _mutation = self.mutation.lock().await;
        self.repository.get(id)?;
        self.disconnect_inner(id).await
    }
    pub(super) async fn disconnect_inner(&self, id: HostId) -> Result<(), AppError> {
        let started = Instant::now();
        let slot = self.slot(id).await;
        let mut data = slot.data.lock().await;
        data.generation += 1;
        data.cancel.cancel();
        data.refreshing = false;
        let transport = data.transport.take();
        let connection_id = data.connection_id.take();
        let state = data.view.state;
        data.view = HostSession::disconnected(id);
        if matches!(
            state,
            ConnectionState::Connecting | ConnectionState::Connected
        ) {
            data.view.state = state
                .transition(ConnectionState::Disconnecting)?
                .transition(ConnectionState::Disconnected)?;
        }
        drop(data);
        if let Some(connection_id) = connection_id {
            self.close_sftp(id, connection_id).await;
            self.terminals
                .disconnect_connection(id, connection_id)
                .await;
        }
        if let Some(transport) = transport {
            transport.disconnect().await?;
        }
        self.record(id, "connection.disconnect", AuditOutcome::Success, started)
    }
    pub async fn reconnect_host(&self, id: HostId) -> Result<(), AppError> {
        self.disconnect_host(id).await?;
        self.connect_host(id).await
    }
    pub async fn refresh_host(&self, id: HostId) -> Result<(), AppError> {
        self.repository.get(id)?;
        let slot = self.slot(id).await;
        let (transport, cancel, generation) = {
            let mut data = slot.data.lock().await;
            if data.view.state != ConnectionState::Connected || data.refreshing {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Connect this host and wait for the current discovery to finish.",
                ));
            }
            let transport = data.transport.clone().ok_or_else(|| {
                AppError::new(ErrorCode::Connection, "SSH session is unavailable.")
            })?;
            data.refreshing = true;
            (transport, data.cancel.clone(), data.generation)
        };
        let started = Instant::now();
        let result = nexus_discovery::discover(transport.as_ref(), cancel.clone()).await;
        let mut data = slot.data.lock().await;
        if data.generation != generation || cancel.is_cancelled() {
            self.record(id, "discovery.refresh", AuditOutcome::Cancelled, started)?;
            return Err(cancelled());
        }
        data.refreshing = false;
        match result {
            Ok(snapshot) => {
                data.view.capabilities = nexus_discovery::capabilities(&snapshot);
                data.view.discovery = Some(snapshot);
                self.record(id, "discovery.refresh", AuditOutcome::Success, started)
            }
            Err(error) => {
                self.record(id, "discovery.refresh", error_outcome(&error), started)?;
                Err(error)
            }
        }
    }
}
