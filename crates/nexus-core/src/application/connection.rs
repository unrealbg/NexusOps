use super::*;

impl Application {
    pub async fn connect_host(&self, id: HostId) -> Result<(), AppError> {
        let (stored, slot, generation, cancel) = {
            let _mutation = self.mutation.lock().await;
            let stored = self.repository.get(id)?;
            let slot = self.slot(id).await;
            let mut data = slot.data.lock().await;
            if matches!(
                data.view.state,
                ConnectionState::Connecting
                    | ConnectionState::Connected
                    | ConnectionState::Disconnecting
            ) {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "This host is already connecting or connected.",
                ));
            }
            let state = data.view.state.transition(ConnectionState::Connecting)?;
            data.cancel.cancel();
            data.cancel = CancellationToken::new();
            data.generation += 1;
            data.view = HostSession::disconnected(id);
            data.view.state = state;
            data.transport = None;
            data.connection_id = None;
            data.refreshing = false;
            let generation = data.generation;
            let cancel = data.cancel.clone();
            drop(data);
            (stored, slot, generation, cancel)
        };
        self.clear_monitor(id).await;
        let started = Instant::now();
        let outcome = self.establish(&stored, cancel.clone()).await;
        let mut data = slot.data.lock().await;
        if data.generation != generation || cancel.is_cancelled() {
            self.record(id, "connection.connect", AuditOutcome::Cancelled, started)?;
            return Err(cancelled());
        }
        match outcome {
            Ok((transport, snapshot, identity)) => {
                data.view.state = data.view.state.transition(ConnectionState::Connected)?;
                data.view.capabilities = nexus_discovery::capabilities(&snapshot);
                data.view.discovery = Some(snapshot);
                data.view.identity = identity;
                data.transport = Some(transport);
                let connection_id = HostSessionId::new();
                data.connection_id = Some(connection_id);
                data.view.host_session_id = Some(connection_id);
                self.record(id, "connection.connect", AuditOutcome::Success, started)?;
                Ok(())
            }
            Err(error) => {
                data.view.state =
                    data.view
                        .state
                        .transition(if error.code == ErrorCode::UnknownHostKey {
                            ConnectionState::AwaitingTrust
                        } else {
                            ConnectionState::Failed
                        })?;
                data.view.error = Some(error.clone());
                data.transport = None;
                tracing::warn!(host_id=%id,code=?error.code,"connection attempt failed");
                self.record(id, "connection.connect", error_outcome(&error), started)?;
                Err(error)
            }
        }
    }
    async fn establish(
        &self,
        stored: &StoredHost,
        cancel: CancellationToken,
    ) -> Result<
        (
            Arc<dyn crate::ConnectedTransport>,
            DiscoverySnapshot,
            Option<HostIdentity>,
        ),
        AppError,
    > {
        let secrets = self.secrets.clone();
        let credential_id = stored.credential_id;
        let credential = tokio::select! {
            biased;
            _=cancel.cancelled()=>return Err(cancelled()),
            value=tokio::task::spawn_blocking(move||secrets.get(credential_id))=>value.map_err(|_|AppError::new(ErrorCode::SecureStorage,"Secure storage task failed."))??,
        };
        credential.validate(stored.host.connection.authentication)?;
        let transport = self
            .provider
            .connect(&stored.host, credential, cancel.clone())
            .await?;
        let snapshot = nexus_discovery::discover(transport.as_ref(), cancel).await?;
        if transport.is_closed() {
            return Err(AppError::new(
                ErrorCode::Connection,
                "The SSH connection closed during discovery.",
            ));
        }
        let identity = self
            .known_hosts
            .fingerprint(
                &stored.host.connection.hostname,
                stored.host.connection.port,
            )?
            .map(|fingerprint| HostIdentity {
                hostname: stored.host.connection.hostname.clone(),
                fingerprint,
            });
        Ok((transport, snapshot, identity))
    }
}
