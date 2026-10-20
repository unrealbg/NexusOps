use nexus_model::{
    AppError, ErrorCode, HostId, HostSessionId, ServiceSnapshot, SystemdStopImpactInspectionId,
};
use nexus_remote_operations::AuthorityBinding;
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::time::Instant;

const INSPECTION_TTL: Duration = Duration::from_secs(120);
const MAX_HOST_SETS: usize = 64;
const MAX_INSPECTIONS_GLOBAL: usize = 4096;

struct InspectionSet {
    binding: AuthorityBinding,
    issued_at: Instant,
    expires_at: Instant,
    entries: HashMap<SystemdStopImpactInspectionId, String>,
}

#[derive(Default)]
struct InspectionState {
    sets: HashMap<HostId, InspectionSet>,
    newest_request: HashMap<HostId, u64>,
    sealed: bool,
}

/// A read-only inspection identity store. Its values cannot be converted to a
/// remote-operation plan ID or consumed by any mutation API.
#[derive(Default)]
pub(crate) struct StopImpactInspectionStore {
    state: Mutex<InspectionState>,
}

impl StopImpactInspectionStore {
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
        remove_expired(&mut state, now);
        if state.sealed || state.newest_request.get(&binding.host_id).copied() != Some(sequence) {
            return Err(stale());
        }
        state.sets.remove(&binding.host_id);
        let mut entries = HashMap::new();
        for entry in &mut snapshot.entries {
            entry.stop_impact_inspection_id = None;
            // Inventory parsing already constrains entries to canonical-looking
            // service unit identities. Revalidate through the typed query value
            // before issuing renderer-visible inspection authority.
            if nexus_operations::SystemdStopImpactQuery::single(entry.unit.clone()).is_err() {
                continue;
            }
            let id = SystemdStopImpactInspectionId::new();
            entry.stop_impact_inspection_id = Some(id);
            entries.insert(id, entry.unit.clone());
        }
        while state.sets.len() >= MAX_HOST_SETS
            || inspection_count(&state).saturating_add(entries.len()) > MAX_INSPECTIONS_GLOBAL
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
            InspectionSet {
                binding,
                issued_at: now,
                expires_at: now + INSPECTION_TTL,
                entries,
            },
        );
        Ok(())
    }

    pub fn resolve(
        &self,
        id: SystemdStopImpactInspectionId,
        expected: AuthorityBinding,
    ) -> Result<String, AppError> {
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
        advance(&mut state, host_id);
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
        advance(&mut state, host_id);
        Ok(())
    }

    pub fn seal_and_revoke_all(&self) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.sealed = true;
        state.sets.clear();
        state.newest_request.clear();
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, InspectionState>, AppError> {
        self.state.lock().map_err(|_| unavailable())
    }
}

fn inspection_count(state: &InspectionState) -> usize {
    state.sets.values().map(|set| set.entries.len()).sum()
}

fn remove_expired(state: &mut InspectionState, now: Instant) {
    state.sets.retain(|_, set| set.expires_at > now);
}

fn advance(state: &mut InspectionState, host_id: HostId) {
    let sequence = state
        .newest_request
        .get(&host_id)
        .copied()
        .unwrap_or_default()
        .saturating_add(1);
    state.newest_request.insert(host_id, sequence);
}

fn stale() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The stop-impact inspection is stale; refresh services and try again.",
    )
}

fn shutting_down() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "NexusOps is shutting down; stop-impact inspection is unavailable.",
    )
}

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Stop-impact inspection authority is unavailable.",
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

    fn snapshot(host: HostId, session: HostSessionId) -> ServiceSnapshot {
        ServiceSnapshot {
            host_id: host,
            host_session_id: session,
            observed_at: "2026-10-09T00:00:00Z".into(),
            entries: vec![ServiceEntry {
                unit: "fixture.service".into(),
                load_state: "loaded".into(),
                active_state: "active".into(),
                sub_state: "running".into(),
                description: "fixture".into(),
                can_start: false,
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
    fn inspection_identity_is_session_generation_and_sequence_bound() {
        let store = StopImpactInspectionStore::default();
        let host = HostId::new();
        let session = HostSessionId::new();
        let stale_sequence = store.begin_request(host).unwrap();
        let current_sequence = store.begin_request(host).unwrap();
        let mut stale_snapshot = snapshot(host, session);
        assert!(
            store
                .publish(
                    binding(host, session, 4),
                    stale_sequence,
                    &mut stale_snapshot
                )
                .is_err()
        );
        let mut current = snapshot(host, session);
        store
            .publish(binding(host, session, 4), current_sequence, &mut current)
            .unwrap();
        let id = current.entries[0].stop_impact_inspection_id.unwrap();
        assert_eq!(
            store.resolve(id, binding(host, session, 4)).unwrap(),
            "fixture.service"
        );
        assert!(store.resolve(id, binding(host, session, 5)).is_err());
        store.revoke_session(host, session).unwrap();
        assert!(store.resolve(id, binding(host, session, 4)).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn inspection_identity_expires_and_shutdown_revokes_without_mutation_authority() {
        let store = StopImpactInspectionStore::default();
        let host = HostId::new();
        let session = HostSessionId::new();
        let sequence = store.begin_request(host).unwrap();
        let mut current = snapshot(host, session);
        store
            .publish(binding(host, session, 8), sequence, &mut current)
            .unwrap();
        let id = current.entries[0].stop_impact_inspection_id.unwrap();
        tokio::time::advance(INSPECTION_TTL + Duration::from_millis(1)).await;
        assert!(store.resolve(id, binding(host, session, 8)).is_err());

        let next = store.begin_request(host).unwrap();
        let mut replacement = snapshot(host, session);
        store
            .publish(binding(host, session, 8), next, &mut replacement)
            .unwrap();
        let replacement_id = replacement.entries[0].stop_impact_inspection_id.unwrap();
        store.seal_and_revoke_all().unwrap();
        assert!(
            store
                .resolve(replacement_id, binding(host, session, 8))
                .is_err()
        );
        assert!(store.begin_request(host).is_err());
    }
}
