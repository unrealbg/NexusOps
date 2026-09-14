use super::*;

impl Application {
    pub async fn trust_host_key(
        &self,
        id: HostId,
        challenge: HostKeyChallenge,
    ) -> Result<(), AppError> {
        let started = Instant::now();
        let _mutation = self.mutation.lock().await;
        self.repository.get(id)?;
        let slot = self.slot(id).await;
        let mut data = slot.data.lock().await;
        if data.view.state != ConnectionState::AwaitingTrust
            || challenge.previous_fingerprint.is_some()
            || data.view.error.as_ref().and_then(|e| e.host_key.as_deref()) != Some(&challenge)
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "This trust request is stale. Connect again to inspect the current server key.",
            ));
        }
        self.known_hosts.trust(&challenge)?;
        data.view.state = data.view.state.transition(ConnectionState::Disconnected)?;
        data.view.error = None;
        self.record(id, "identity.trust", AuditOutcome::Success, started)
    }
}
