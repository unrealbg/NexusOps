use crate::{
    AuthorityBinding, ConsumedAuthority, NativeOperation, PlanDraft, PlanReceipt,
    RemoteOperationPlanId, authority::StoredAuthority,
};
use nexus_model::{AppError, ErrorCode, HostId, HostSessionId};
use std::{
    any::TypeId,
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::time::Instant;

pub const PLAN_TTL: Duration = Duration::from_secs(120);

#[derive(Default)]
struct StoreState {
    pending: HashMap<RemoteOperationPlanId, StoredAuthority>,
    by_host: HashMap<HostId, RemoteOperationPlanId>,
    sealed: bool,
}

pub struct AuthorityStore {
    state: Arc<Mutex<StoreState>>,
    ttl: Duration,
}

impl Default for AuthorityStore {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            ttl: PLAN_TTL,
        }
    }
}

impl AuthorityStore {
    #[cfg(test)]
    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self {
            state: Arc::default(),
            ttl,
        }
    }

    /// Publishes a plan only when the backend binding captured after planning
    /// still equals the binding used to create the draft.
    pub fn insert<O: NativeOperation>(
        &self,
        draft: PlanDraft<O>,
        current: AuthorityBinding,
    ) -> Result<PlanReceipt, AppError> {
        if draft.binding != current {
            return Err(stale());
        }
        let issued_at = Instant::now();
        let expires_at = issued_at + self.ttl;
        let id = RemoteOperationPlanId::new();
        let binding = draft.binding;
        let risk = O::RISK;
        let authority = StoredAuthority::new(id, draft, issued_at, expires_at);
        let superseded = {
            let mut state = self.lock()?;
            if state.sealed {
                return Err(shutting_down());
            }
            remove_expired(&mut state, issued_at);
            let superseded = state.by_host.remove(&binding.host_id);
            if let Some(old) = superseded {
                state.pending.remove(&old);
            }
            state.pending.insert(id, authority);
            state.by_host.insert(binding.host_id, id);
            superseded
        };
        schedule_expiry(Arc::downgrade(&self.state), id, expires_at);
        Ok(PlanReceipt {
            id,
            binding,
            risk,
            issued_at,
            expires_at,
            superseded,
        })
    }

    /// Atomically removes a matching plan. Revalidation and execution happen
    /// after this call; the consumed authority can never be returned to pending.
    pub fn consume<O: NativeOperation>(
        &self,
        id: RemoteOperationPlanId,
        expected: AuthorityBinding,
    ) -> Result<ConsumedAuthority<O>, AppError> {
        let now = Instant::now();
        let stored = {
            let mut state = self.lock()?;
            if state
                .pending
                .get(&id)
                .is_some_and(|value| value.expires_at <= now)
            {
                remove_id(&mut state, id);
                return Err(expired());
            }
            let current = state.pending.get(&id).ok_or_else(missing)?;
            if current.binding != expected {
                return Err(stale());
            }
            if current.operation_type != TypeId::of::<O>() || current.risk != O::RISK {
                return Err(policy());
            }
            remove_id(&mut state, id).ok_or_else(missing)?
        };
        stored.into_typed::<O>().ok_or_else(policy)
    }

    /// Discard and consume use the same synchronization boundary; only one wins.
    pub fn discard<O: NativeOperation>(
        &self,
        id: RemoteOperationPlanId,
        expected: AuthorityBinding,
    ) -> Result<bool, AppError> {
        let now = Instant::now();
        let mut state = self.lock()?;
        if state
            .pending
            .get(&id)
            .is_some_and(|value| value.expires_at <= now)
        {
            remove_id(&mut state, id);
            return Ok(false);
        }
        let Some(current) = state.pending.get(&id) else {
            return Ok(false);
        };
        if current.binding != expected {
            return Err(stale());
        }
        if current.operation_type != TypeId::of::<O>() || current.risk != O::RISK {
            return Err(policy());
        }
        Ok(remove_id(&mut state, id).is_some())
    }

    pub fn revoke_host(&self, host_id: HostId) -> Result<bool, AppError> {
        let mut state = self.lock()?;
        let Some(id) = state.by_host.remove(&host_id) else {
            return Ok(false);
        };
        Ok(state.pending.remove(&id).is_some())
    }

    pub fn revoke_session(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<bool, AppError> {
        let mut state = self.lock()?;
        let Some(id) = state.by_host.get(&host_id).copied() else {
            return Ok(false);
        };
        if state
            .pending
            .get(&id)
            .is_some_and(|value| value.binding.host_session_id == host_session_id)
        {
            Ok(remove_id(&mut state, id).is_some())
        } else {
            Ok(false)
        }
    }

    pub fn revoke_all(&self) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.pending.clear();
        state.by_host.clear();
        Ok(())
    }

    /// Atomically prevents later publication and revokes every live authority.
    pub fn seal_and_revoke_all(&self) -> Result<(), AppError> {
        let mut state = self.lock()?;
        state.sealed = true;
        state.pending.clear();
        state.by_host.clear();
        Ok(())
    }

    pub fn is_sealed(&self) -> Result<bool, AppError> {
        Ok(self.lock()?.sealed)
    }

    pub fn pending_count(&self) -> Result<usize, AppError> {
        let mut state = self.lock()?;
        remove_expired(&mut state, Instant::now());
        Ok(state.pending.len())
    }

    pub fn contains(&self, id: RemoteOperationPlanId) -> Result<bool, AppError> {
        let mut state = self.lock()?;
        remove_expired(&mut state, Instant::now());
        Ok(state.pending.contains_key(&id))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StoreState>, AppError> {
        self.state.lock().map_err(|_| unavailable())
    }

    #[cfg(test)]
    pub(crate) fn corrupt_risk(&self, id: RemoteOperationPlanId, risk: nexus_model::OperationRisk) {
        self.state
            .lock()
            .unwrap()
            .pending
            .get_mut(&id)
            .unwrap()
            .risk = risk;
    }
}

fn remove_id(state: &mut StoreState, id: RemoteOperationPlanId) -> Option<StoredAuthority> {
    let removed = state.pending.remove(&id)?;
    if state.by_host.get(&removed.binding.host_id) == Some(&id) {
        state.by_host.remove(&removed.binding.host_id);
    }
    Some(removed)
}

fn remove_expired(state: &mut StoreState, now: Instant) {
    let expired = state
        .pending
        .iter()
        .filter_map(|(id, value)| (value.expires_at <= now).then_some(*id))
        .collect::<Vec<_>>();
    for id in expired {
        remove_id(state, id);
    }
}

fn schedule_expiry(state: Weak<Mutex<StoreState>>, id: RemoteOperationPlanId, expires: Instant) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        tokio::time::sleep_until(expires).await;
        if let Some(state) = state.upgrade()
            && let Ok(mut state) = state.lock()
            && state
                .pending
                .get(&id)
                .is_some_and(|value| value.expires_at == expires)
        {
            remove_id(&mut state, id);
        }
    });
}

fn unavailable() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Remote operation authority is unavailable.",
    )
}

fn stale() -> AppError {
    AppError::new(ErrorCode::Conflict, "The remote operation plan is stale.")
}

fn expired() -> AppError {
    AppError::new(ErrorCode::Conflict, "The remote operation plan expired.")
}

fn missing() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The remote operation plan is missing or was already used.",
    )
}

fn policy() -> AppError {
    AppError::new(
        ErrorCode::Policy,
        "The remote operation identity is invalid.",
    )
}

fn shutting_down() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "NexusOps is shutting down; create a fresh plan after restart.",
    )
}
