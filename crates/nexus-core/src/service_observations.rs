use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, ServiceObservationId, ServiceSnapshot,
};
use nexus_remote_operations::{
    AuthorityBinding, SystemdReloadPreconditions, SystemdResetFailedPreconditions,
    SystemdServiceUnitName, SystemdStartPreconditions, SystemdTryRestartPreconditions,
};
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::time::Instant;

pub(crate) const SERVICE_OBSERVATION_TTL: Duration = Duration::from_secs(120);
const MAX_HOST_SETS: usize = 64;
const MAX_ACTIONABLE_GLOBAL: usize = 4096;

#[derive(Clone)]
pub(crate) struct ResetFailedObservation {
    pub unit: SystemdServiceUnitName,
    pub preconditions: SystemdResetFailedPreconditions,
}

#[derive(Clone)]
pub(crate) struct TryRestartObservation {
    pub unit: SystemdServiceUnitName,
    pub preconditions: SystemdTryRestartPreconditions,
}

#[derive(Clone)]
pub(crate) struct ReloadObservation {
    pub unit: SystemdServiceUnitName,
    pub preconditions: SystemdReloadPreconditions,
}

#[derive(Clone)]
pub(crate) struct StartObservation {
    pub unit: SystemdServiceUnitName,
    pub preconditions: SystemdStartPreconditions,
}

#[derive(Clone)]
enum ActionableServiceObservation {
    ResetFailed(ResetFailedObservation),
    TryRestart(TryRestartObservation),
    Reload(ReloadObservation),
    Start(StartObservation),
}

struct ObservationSet {
    binding: AuthorityBinding,
    issued_at: Instant,
    expires_at: Instant,
    entries: HashMap<ServiceObservationId, ActionableServiceObservation>,
}

#[derive(Default)]
struct ObservationState {
    sets: HashMap<HostId, ObservationSet>,
    newest_request: HashMap<HostId, u64>,
    sealed: bool,
}

#[derive(Default)]
pub(crate) struct ServiceObservationStore {
    state: Mutex<ObservationState>,
}

impl ServiceObservationStore {
    pub fn begin_request(&self, host_id: HostId) -> Result<u64, AppError> {
        let mut state = self.lock()?;
        if state.sealed {
            return Err(shutting_down());
        }
        let sequence = state
            .newest_request
            .get(&host_id)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        state.newest_request.insert(host_id, sequence);
        Ok(sequence)
    }

    /// Called only while the host lifecycle gate protects the final binding check.
    pub fn publish(
        &self,
        binding: AuthorityBinding,
        sequence: u64,
        snapshot: &mut ServiceSnapshot,
    ) -> Result<(), AppError> {
        if snapshot.host_id != binding.host_id
            || snapshot.host_session_id != binding.host_session_id
            || snapshot.entries.len() > 512
        {
            return Err(stale());
        }
        let now = Instant::now();
        let mut state = self.lock()?;
        if state.sealed || state.newest_request.get(&binding.host_id).copied() != Some(sequence) {
            return Err(stale());
        }
        remove_expired(&mut state, now);
        state.sets.remove(&binding.host_id);

        let mut entries = HashMap::new();
        for entry in &mut snapshot.entries {
            entry.reset_failed_observation_id = None;
            entry.try_restart_observation_id = None;
            entry.reload_observation_id = None;
            entry.start_observation_id = None;
            let Ok(unit) = SystemdServiceUnitName::parse(&entry.unit) else {
                continue;
            };
            if SystemdResetFailedPreconditions.matches(
                &entry.load_state,
                &entry.active_state,
                &entry.sub_state,
            ) {
                let id = ServiceObservationId::new();
                entry.reset_failed_observation_id = Some(id);
                entries.insert(
                    id,
                    ActionableServiceObservation::ResetFailed(ResetFailedObservation {
                        unit: unit.clone(),
                        preconditions: SystemdResetFailedPreconditions,
                    }),
                );
            }
            if SystemdTryRestartPreconditions.matches(
                &entry.load_state,
                &entry.active_state,
                &entry.sub_state,
            ) {
                let id = ServiceObservationId::new();
                entry.try_restart_observation_id = Some(id);
                entries.insert(
                    id,
                    ActionableServiceObservation::TryRestart(TryRestartObservation {
                        unit: unit.clone(),
                        preconditions: SystemdTryRestartPreconditions,
                    }),
                );
            }
            if SystemdReloadPreconditions.matches(
                &entry.load_state,
                &entry.active_state,
                &entry.sub_state,
                entry.can_reload,
            ) {
                let id = ServiceObservationId::new();
                entry.reload_observation_id = Some(id);
                entries.insert(
                    id,
                    ActionableServiceObservation::Reload(ReloadObservation {
                        unit: unit.clone(),
                        preconditions: SystemdReloadPreconditions,
                    }),
                );
            }
            if SystemdStartPreconditions.matches(
                &entry.load_state,
                &entry.active_state,
                &entry.sub_state,
                entry.can_start,
            ) {
                let id = ServiceObservationId::new();
                entry.start_observation_id = Some(id);
                entries.insert(
                    id,
                    ActionableServiceObservation::Start(StartObservation {
                        unit: unit.clone(),
                        preconditions: SystemdStartPreconditions,
                    }),
                );
            }
        }

        while state.sets.len() >= MAX_HOST_SETS
            || actionable_count(&state).saturating_add(entries.len()) > MAX_ACTIONABLE_GLOBAL
        {
            let Some(oldest) = state
                .sets
                .iter()
                .min_by_key(|(_, set)| set.issued_at)
                .map(|(host, _)| *host)
            else {
                break;
            };
            state.sets.remove(&oldest);
        }
        state.sets.insert(
            binding.host_id,
            ObservationSet {
                binding,
                issued_at: now,
                expires_at: now + SERVICE_OBSERVATION_TTL,
                entries,
            },
        );
        Ok(())
    }

    pub fn resolve_reset_failed(
        &self,
        id: ServiceObservationId,
        expected: AuthorityBinding,
    ) -> Result<ResetFailedObservation, AppError> {
        match self.resolve(id, expected)? {
            ActionableServiceObservation::ResetFailed(value) => Ok(value),
            ActionableServiceObservation::TryRestart(_) => Err(stale()),
            ActionableServiceObservation::Reload(_) => Err(stale()),
            ActionableServiceObservation::Start(_) => Err(stale()),
        }
    }

    pub fn resolve_try_restart(
        &self,
        id: ServiceObservationId,
        expected: AuthorityBinding,
    ) -> Result<TryRestartObservation, AppError> {
        match self.resolve(id, expected)? {
            ActionableServiceObservation::TryRestart(value) => Ok(value),
            ActionableServiceObservation::ResetFailed(_) => Err(stale()),
            ActionableServiceObservation::Reload(_) => Err(stale()),
            ActionableServiceObservation::Start(_) => Err(stale()),
        }
    }

    pub fn resolve_reload(
        &self,
        id: ServiceObservationId,
        expected: AuthorityBinding,
    ) -> Result<ReloadObservation, AppError> {
        match self.resolve(id, expected)? {
            ActionableServiceObservation::Reload(value) => Ok(value),
            ActionableServiceObservation::ResetFailed(_)
            | ActionableServiceObservation::TryRestart(_)
            | ActionableServiceObservation::Start(_) => Err(stale()),
        }
    }

    pub fn resolve_start(
        &self,
        id: ServiceObservationId,
        expected: AuthorityBinding,
    ) -> Result<StartObservation, AppError> {
        match self.resolve(id, expected)? {
            ActionableServiceObservation::Start(value) => Ok(value),
            ActionableServiceObservation::ResetFailed(_)
            | ActionableServiceObservation::TryRestart(_)
            | ActionableServiceObservation::Reload(_) => Err(stale()),
        }
    }

    fn resolve(
        &self,
        id: ServiceObservationId,
        expected: AuthorityBinding,
    ) -> Result<ActionableServiceObservation, AppError> {
        let now = Instant::now();
        let mut state = self.lock()?;
        remove_expired(&mut state, now);
        let set = state.sets.get(&expected.host_id).ok_or_else(stale)?;
        if set.binding != expected {
            return Err(stale());
        }
        set.entries.get(&id).cloned().ok_or_else(stale)
    }

    pub fn revoke_host(&self, host_id: HostId) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.sets.remove(&host_id);
        let next = state
            .newest_request
            .get(&host_id)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        state.newest_request.insert(host_id, next);
        Ok(())
    }

    pub fn revoke_session(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<(), AppError> {
        let mut state = self.lock()?;
        if state
            .sets
            .get(&host_id)
            .is_some_and(|set| set.binding.host_session_id == host_session_id)
        {
            state.sets.remove(&host_id);
        }
        let next = state
            .newest_request
            .get(&host_id)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        state.newest_request.insert(host_id, next);
        Ok(())
    }

    pub fn seal_and_revoke_all(&self) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.sealed = true;
        state.sets.clear();
        state.newest_request.clear();
        Ok(())
    }

    #[cfg(test)]
    pub fn contains(&self, id: ServiceObservationId) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.sets.values().any(|set| set.entries.contains_key(&id)))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ObservationState>, AppError> {
        self.state.lock().map_err(|_| unavailable())
    }
}

fn actionable_count(state: &ObservationState) -> usize {
    state.sets.values().map(|set| set.entries.len()).sum()
}

fn remove_expired(state: &mut ObservationState, now: Instant) {
    state.sets.retain(|_, set| set.expires_at > now);
}

fn stale() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The service observation is stale; refresh services and try again.",
    )
}

fn shutting_down() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "NexusOps is shutting down; service observations are unavailable.",
    )
}

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Service observation authority is unavailable.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_model::ServiceEntry;

    fn binding(host: HostId, session: HostSessionId, generation: u64) -> AuthorityBinding {
        AuthorityBinding {
            host_id: host,
            host_session_id: session,
            generation,
        }
    }

    fn snapshot(host: HostId, session: HostSessionId, unit: &str) -> ServiceSnapshot {
        ServiceSnapshot {
            host_id: host,
            host_session_id: session,
            observed_at: "2026-10-04T00:00:00Z".into(),
            entries: vec![ServiceEntry {
                unit: unit.into(),
                load_state: "loaded".into(),
                active_state: "failed".into(),
                sub_state: "failed".into(),
                description: "fixture".into(),
                can_start: true,
                can_reload: false,
                reset_failed_observation_id: None,
                try_restart_observation_id: None,
                reload_observation_id: None,
                start_observation_id: None,
                stop_impact_inspection_id: None,
            }],
        }
    }

    #[test]
    fn newer_request_prevents_delayed_older_publication() {
        let store = ServiceObservationStore::default();
        let host = HostId::new();
        let session = HostSessionId::new();
        let first = store.begin_request(host).unwrap();
        let second = store.begin_request(host).unwrap();
        let mut newer = snapshot(host, session, "newer.service");
        store
            .publish(binding(host, session, 1), second, &mut newer)
            .unwrap();
        let newer_id = newer.entries[0].reset_failed_observation_id.unwrap();
        let mut older = snapshot(host, session, "older.service");
        assert!(
            store
                .publish(binding(host, session, 1), first, &mut older)
                .is_err()
        );
        assert!(store.contains(newer_id));
        assert!(older.entries[0].reset_failed_observation_id.is_none());
    }

    #[test]
    fn only_exact_actionable_rows_receive_ids_and_revocation_is_terminal() {
        let store = ServiceObservationStore::default();
        let host = HostId::new();
        let session = HostSessionId::new();
        let sequence = store.begin_request(host).unwrap();
        let mut value = snapshot(host, session, "worker@1.service");
        value.entries.push(ServiceEntry {
            unit: "active.service".into(),
            load_state: "loaded".into(),
            active_state: "active".into(),
            sub_state: "running".into(),
            description: "active".into(),
            can_start: true,
            can_reload: true,
            reset_failed_observation_id: None,
            try_restart_observation_id: None,
            reload_observation_id: None,
            start_observation_id: None,
            stop_impact_inspection_id: None,
        });
        value.entries.push(ServiceEntry {
            unit: "inactive.service".into(),
            load_state: "loaded".into(),
            active_state: "inactive".into(),
            sub_state: "dead".into(),
            description: "inactive".into(),
            can_start: true,
            can_reload: false,
            reset_failed_observation_id: None,
            try_restart_observation_id: None,
            reload_observation_id: None,
            start_observation_id: None,
            stop_impact_inspection_id: None,
        });
        value.entries.push(ServiceEntry {
            unit: "bad;unit.service".into(),
            load_state: "loaded".into(),
            active_state: "active".into(),
            sub_state: "running".into(),
            description: "malformed".into(),
            can_start: true,
            can_reload: true,
            reset_failed_observation_id: None,
            try_restart_observation_id: None,
            reload_observation_id: None,
            start_observation_id: None,
            stop_impact_inspection_id: None,
        });
        value.entries.push(ServiceEntry {
            unit: "inactive-disabled.service".into(),
            load_state: "loaded".into(),
            active_state: "inactive".into(),
            sub_state: "dead".into(),
            description: "inactive but not startable".into(),
            can_start: false,
            can_reload: false,
            reset_failed_observation_id: None,
            try_restart_observation_id: None,
            reload_observation_id: None,
            start_observation_id: None,
            stop_impact_inspection_id: None,
        });
        let owner = binding(host, session, 7);
        store.publish(owner, sequence, &mut value).unwrap();
        assert_eq!(actionable_count(&store.state.lock().unwrap()), 4);
        let id = value.entries[0].reset_failed_observation_id.unwrap();
        assert!(value.entries[1].reset_failed_observation_id.is_none());
        let restart_id = value.entries[1].try_restart_observation_id.unwrap();
        let reload_id = value.entries[1].reload_observation_id.unwrap();
        let start_id = value.entries[2].start_observation_id.unwrap();
        assert!(value.entries[0].try_restart_observation_id.is_none());
        assert!(value.entries[0].start_observation_id.is_none());
        assert!(value.entries[1].start_observation_id.is_none());
        assert!(value.entries[3].start_observation_id.is_none());
        assert!(value.entries[4].start_observation_id.is_none());
        for entry in &value.entries[2..] {
            assert!(entry.reset_failed_observation_id.is_none());
            assert!(entry.try_restart_observation_id.is_none());
            assert!(entry.reload_observation_id.is_none());
        }
        assert_eq!(
            store.resolve_reset_failed(id, owner).unwrap().unit.as_str(),
            "worker@1.service"
        );
        assert_eq!(
            store
                .resolve_try_restart(restart_id, owner)
                .unwrap()
                .unit
                .as_str(),
            "active.service"
        );
        assert!(store.resolve_try_restart(id, owner).is_err());
        assert!(store.resolve_reset_failed(restart_id, owner).is_err());
        assert_eq!(
            store
                .resolve_reload(reload_id, owner)
                .unwrap()
                .unit
                .as_str(),
            "active.service"
        );
        assert!(store.resolve_reload(restart_id, owner).is_err());
        assert!(store.resolve_try_restart(reload_id, owner).is_err());
        assert!(store.resolve_reload(id, owner).is_err());
        assert!(store.resolve_reset_failed(reload_id, owner).is_err());
        assert_eq!(
            store.resolve_start(start_id, owner).unwrap().unit.as_str(),
            "inactive.service"
        );
        assert!(store.resolve_start(id, owner).is_err());
        assert!(store.resolve_start(restart_id, owner).is_err());
        assert!(store.resolve_start(reload_id, owner).is_err());
        assert!(store.resolve_reset_failed(start_id, owner).is_err());
        assert!(store.resolve_try_restart(start_id, owner).is_err());
        assert!(store.resolve_reload(start_id, owner).is_err());
        store.revoke_session(host, session).unwrap();
        assert!(store.resolve_reset_failed(id, owner).is_err());
        assert!(store.resolve_try_restart(restart_id, owner).is_err());
        assert!(store.resolve_reload(reload_id, owner).is_err());
        assert!(store.resolve_start(start_id, owner).is_err());
    }
}
