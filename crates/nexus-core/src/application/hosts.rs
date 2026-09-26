use super::*;

impl Application {
    pub fn list_hosts(&self) -> Result<Vec<Host>, AppError> {
        self.repository.list()
    }

    pub async fn save_host(
        &self,
        input: HostInput,
        credential: Option<CredentialInput>,
    ) -> Result<Host, AppError> {
        let started = Instant::now();
        let _mutation = self.mutation.lock().await;
        let previous = input.id.map(|id| self.repository.get(id)).transpose()?;
        let host = input.into_host()?;
        let slot = self.slot(host.id).await;
        let data = slot.data.lock().await;
        if matches!(
            data.view.state,
            ConnectionState::Connecting
                | ConnectionState::Connected
                | ConnectionState::Disconnecting
        ) {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Disconnect this host before editing its configuration.",
            ));
        }
        drop(data);
        if credential.is_none()
            && previous
                .as_ref()
                .is_none_or(|p| p.host.connection.authentication != host.connection.authentication)
        {
            return Err(AppError::new(
                ErrorCode::Validation,
                "Credentials are required for a new host or authentication method.",
            ));
        }
        let credential_id = if let Some(input) = credential {
            let value = input.into_credential(host.connection.authentication)?;
            let id = HostId::new();
            let secrets = self.secrets.clone();
            tokio::task::spawn_blocking(move || secrets.put(id, &value))
                .await
                .map_err(|_| {
                    AppError::new(ErrorCode::SecureStorage, "Secure storage task failed.")
                })??;
            id
        } else {
            previous
                .as_ref()
                .ok_or_else(|| AppError::new(ErrorCode::Validation, "Credentials are required."))?
                .credential_id
        };
        let stored = StoredHost {
            host: host.clone(),
            credential_id,
        };
        if let Err(error) = self.repository.save(&stored) {
            if previous
                .as_ref()
                .is_none_or(|p| p.credential_id != credential_id)
            {
                self.remove_secret(credential_id).await?;
            }
            return Err(error);
        }
        // Invalidate pending trust immediately after the metadata commit, even if cleanup fails.
        {
            let mut data = slot.data.lock().await;
            data.generation += 1;
            data.cancel.cancel();
            data.view = HostSession::disconnected(host.id);
            data.connection_id = None;
        }
        self.clear_monitor(host.id).await;
        if previous.is_some() {
            self.terminals.remove_host(host.id).await;
        }
        // A failed cleanup leaves only an encrypted orphan, never a dangling metadata reference.
        if let Some(old) = previous
            && old.credential_id != credential_id
        {
            self.remove_secret(old.credential_id).await?;
        }
        self.record(host.id, "host.save", AuditOutcome::Success, started)?;
        Ok(host)
    }
    async fn remove_secret(&self, id: HostId) -> Result<(), AppError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.delete(id))
            .await
            .map_err(|_| AppError::new(ErrorCode::SecureStorage, "Secure storage task failed."))?
    }
    pub async fn delete_host(&self, id: HostId) -> Result<(), AppError> {
        let started = Instant::now();
        let _mutation = self.mutation.lock().await;
        let stored = self.repository.get(id)?;
        self.disconnect_inner(id).await?;
        // Metadata first: a crash can leave encrypted garbage, never a host referring to deleted secrets.
        self.repository.delete(id)?;
        self.remove_secret(stored.credential_id).await?;
        self.sessions.lock().await.remove(&id);
        self.remove_monitor(id).await;
        self.terminals.remove_host(id).await;
        self.record(id, "host.delete", AuditOutcome::Success, started)
    }
}
