use super::*;

pub(super) struct DisconnectWork {
    started: Instant,
    slot: Arc<SessionSlot>,
    connection_id: Option<HostSessionId>,
}

impl Application {
    pub async fn get_session(&self, id: HostId) -> Result<HostSession, AppError> {
        self.repository.get(id)?;
        let slot = self.slot(id).await;
        let needs_remote_close = {
            let data = slot.data.lock().await;
            data.transport
                .as_ref()
                .is_some_and(|transport| transport.is_closed())
                && data.view.state == ConnectionState::Connected
        };
        if !needs_remote_close {
            return Ok(slot.data.lock().await.view.clone());
        }
        let _host_operation = self.remote_operations.lifecycle_guard(id).await?;
        let mut data = slot.data.lock().await;
        let closed_connection = if data.transport.as_ref().is_some_and(|t| t.is_closed())
            && data.view.state == ConnectionState::Connected
        {
            data.generation += 1;
            data.refreshing = false;
            data.cancel.cancel();
            data.transport = None;
            let connection_id = data.connection_id.take();
            data.view.host_session_id = None;
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
        if closed_connection.is_some() {
            self.clear_monitor(id).await;
        }
        if let Some(connection_id) = closed_connection {
            self.remote_operations.revoke_session(id, connection_id)?;
            self.service_observations
                .revoke_session(id, connection_id)?;
            self.close_sftp(id, connection_id).await?;
            self.terminals
                .disconnect_connection(id, connection_id)
                .await;
        }
        Ok(view)
    }
    pub async fn disconnect_host(&self, id: HostId) -> Result<(), AppError> {
        let _host_operation = self.remote_operations.lifecycle_guard(id).await?;
        let work = {
            let _mutation = self.mutation.lock().await;
            self.repository.get(id)?;
            self.prepare_disconnect(id).await?
        };
        self.finish_disconnect(id, work).await
    }

    pub(super) async fn prepare_disconnect(&self, id: HostId) -> Result<DisconnectWork, AppError> {
        self.remote_operations.revoke_host(id)?;
        self.service_observations.revoke_host(id)?;
        self.rotation_plans.lock().await.remove(&id);
        let started = Instant::now();
        let slot = self.slot(id).await;
        let mut data = slot.data.lock().await;
        data.generation += 1;
        data.refreshing = false;
        let connection_id = data.connection_id;
        let state = data.view.state;
        if state == ConnectionState::Connecting {
            data.cancel.cancel();
        }
        if matches!(
            state,
            ConnectionState::Connecting | ConnectionState::Connected
        ) {
            data.view.state = state.transition(ConnectionState::Disconnecting)?;
        }
        drop(data);
        self.clear_monitor(id).await;
        Ok(DisconnectWork {
            started,
            slot,
            connection_id,
        })
    }

    pub(super) async fn finish_disconnect(
        &self,
        id: HostId,
        work: DisconnectWork,
    ) -> Result<(), AppError> {
        if let Some(connection_id) = work.connection_id {
            if let Err(error) = self.close_sftp(id, connection_id).await {
                work.slot.data.lock().await.view.error = Some(error.clone());
                return Err(error);
            }
            self.terminals
                .disconnect_connection(id, connection_id)
                .await;
        }
        let mut data = work.slot.data.lock().await;
        data.cancel.cancel();
        let transport = data.transport.take();
        data.connection_id = None;
        data.view = HostSession::disconnected(id);
        drop(data);
        if let Some(transport) = transport {
            transport.disconnect().await?;
        }
        self.record(
            id,
            "connection.disconnect",
            AuditOutcome::Success,
            work.started,
        )
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
