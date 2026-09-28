use super::*;
use crate::sessions::SessionData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ROTATION_TTL: Duration = Duration::from_secs(5 * 60);

fn rotation_conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "This SSH key change is stale. Connect again and review a new rotation plan.",
    )
}

fn stored_host(id: HostId, app: &Application) -> Result<Host, AppError> {
    let host = app
        .repository
        .get(id)
        .map_err(|error| {
            if error.code == ErrorCode::NotFound {
                rotation_conflict()
            } else {
                error
            }
        })?
        .host;
    host.connection.validate().map_err(|_| {
        AppError::new(
            ErrorCode::Persistence,
            "The stored SSH endpoint configuration is invalid.",
        )
    })?;
    Ok(host)
}

fn changed_challenge(host: &Host, data: &SessionData) -> Result<HostKeyChallenge, AppError> {
    if data.view.host_id != host.id
        || data.view.state != ConnectionState::Failed
        || data.view.host_session_id.is_some()
        || data.connection_id.is_some()
        || data.transport.is_some()
    {
        return Err(rotation_conflict());
    }
    let error = data.view.error.as_ref().ok_or_else(rotation_conflict)?;
    if error.code != ErrorCode::ChangedHostKey {
        return Err(rotation_conflict());
    }
    let challenge = error.host_key.as_deref().ok_or_else(rotation_conflict)?;
    let canonical = KnownHosts::canonical_hostname(&host.connection.hostname, host.connection.port)
        .map_err(|_| rotation_conflict())?;
    if challenge.hostname != canonical
        || challenge.port != host.connection.port
        || challenge.previous_fingerprint.is_none()
    {
        return Err(rotation_conflict());
    }
    Ok(challenge.clone())
}

impl Application {
    /// Creates display data from the current backend-owned failed handshake only.
    pub async fn plan_host_key_rotation(
        &self,
        id: HostId,
    ) -> Result<HostKeyRotationPlan, AppError> {
        let _mutation = self.mutation.lock().await;
        // A re-plan attempt retires the preceding authority for this host.
        self.rotation_plans.lock().await.remove(&id);
        let host = stored_host(id, self)?;
        let slot = self.slot(id).await;
        let data = slot.data.lock().await;
        let challenge = changed_challenge(&host, &data)?;
        let current = self
            .known_hosts
            .fingerprint(&host.connection.hostname, host.connection.port)?
            .ok_or_else(rotation_conflict)?;
        if challenge.previous_fingerprint.as_deref() != Some(current.sha256.as_str()) {
            return Err(rotation_conflict());
        }
        let presented = HostFingerprint {
            algorithm: challenge.algorithm,
            sha256: challenge.fingerprint,
        };
        // Reuse the exact SSH verifier to validate the presented format and changed-key
        // relationship; do not return its fingerprint-bearing challenge as an error.
        match self
            .known_hosts
            .verify(&host.connection.hostname, host.connection.port, &presented)
        {
            Err(error) if error.code == ErrorCode::ChangedHostKey => {}
            Err(error) if error.code == ErrorCode::Persistence => return Err(error),
            _ => return Err(rotation_conflict()),
        }
        let expires_at_unix_ms = SystemTime::now()
            .checked_add(ROTATION_TTL)
            .and_then(|deadline| deadline.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
            .ok_or_else(|| {
                AppError::new(ErrorCode::Persistence, "The local clock is unavailable.")
            })?;
        let plan = HostKeyRotationPlan {
            id: HostKeyRotationPlanId::new(),
            host_id: id,
            hostname: KnownHosts::canonical_hostname(
                &host.connection.hostname,
                host.connection.port,
            )?,
            port: host.connection.port,
            current_fingerprint: current,
            presented_fingerprint: presented,
            expires_at_unix_ms,
        };
        self.rotation_plans.lock().await.insert(
            id,
            RotationAuthority {
                plan: plan.clone(),
                generation: data.generation,
                expires_at: Instant::now() + ROTATION_TTL,
            },
        );
        Ok(plan)
    }

    /// Consumes backend-owned authority and changes only the local endpoint pin.
    pub async fn execute_host_key_rotation(
        &self,
        id: HostId,
        plan_id: HostKeyRotationPlanId,
    ) -> Result<(), AppError> {
        let started = Instant::now();
        let _mutation = self.mutation.lock().await;
        let authority = {
            let mut plans = self.rotation_plans.lock().await;
            let current = plans.get(&id).ok_or_else(rotation_conflict)?;
            if current.plan.id != plan_id {
                return Err(rotation_conflict());
            }
            plans.remove(&id).ok_or_else(rotation_conflict)?
        };
        let result = async {
            if Instant::now() >= authority.expires_at {
                return Err(rotation_conflict());
            }
            let host = stored_host(id, self)?;
            let plan = &authority.plan;
            let canonical =
                KnownHosts::canonical_hostname(&host.connection.hostname, host.connection.port)
                    .map_err(|_| rotation_conflict())?;
            if plan.host_id != id || plan.hostname != canonical || plan.port != host.connection.port
            {
                return Err(rotation_conflict());
            }
            let slot = self.slot(id).await;
            let mut data = slot.data.lock().await;
            if data.generation != authority.generation {
                return Err(rotation_conflict());
            }
            let challenge = changed_challenge(&host, &data)?;
            if challenge.hostname != plan.hostname
                || challenge.port != plan.port
                || challenge.algorithm != plan.presented_fingerprint.algorithm
                || challenge.fingerprint != plan.presented_fingerprint.sha256
                || challenge.previous_fingerprint.as_deref()
                    != Some(plan.current_fingerprint.sha256.as_str())
            {
                return Err(rotation_conflict());
            }
            let stored = self
                .known_hosts
                .fingerprint(&host.connection.hostname, host.connection.port)?;
            if stored.as_ref() != Some(&plan.current_fingerprint) {
                return Err(rotation_conflict());
            }
            self.known_hosts.rotate(
                &plan.hostname,
                plan.port,
                &plan.current_fingerprint,
                &plan.presented_fingerprint,
            )?;
            data.generation += 1;
            data.cancel.cancel();
            data.refreshing = false;
            data.transport = None;
            data.connection_id = None;
            data.view = HostSession::disconnected(id);
            Ok(())
        }
        .await;
        match result {
            Ok(()) => self.record(id, "identity.rotate", AuditOutcome::Success, started),
            Err(error) => {
                self.record(id, "identity.rotate", AuditOutcome::Failed, started)?;
                Err(error)
            }
        }
    }
}
