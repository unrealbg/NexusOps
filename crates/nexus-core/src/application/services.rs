use super::*;
use nexus_remote_operations::{
    AuthorityBinding, AuthorityRevalidator, ConsumedAuthority, ExecutionTerminal,
    MutationTransportOutcome, PlanDraft, RemoteOperationOutcome, SystemdResetFailed,
    SystemdResetFailedPreconditions, SystemdServiceUnitName,
};
use tokio::sync::OwnedMutexGuard;

const RESET_FAILED_EFFECT: &str = "Clears the selected service's failed state and rate-limit/restart counters. It does not intentionally start or stop the service, but it can affect later service behavior.";

fn stale_session() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The service request does not belong to the active host session.",
    )
}

fn stale_observation() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "The service observation is stale; refresh services and try again.",
    )
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "A service request is already in progress or the service limit is reached.",
    )
}

fn exact_failed_service<'a>(
    snapshot: &'a ServiceSnapshot,
    unit: &SystemdServiceUnitName,
    preconditions: &SystemdResetFailedPreconditions,
) -> Result<&'a ServiceEntry, AppError> {
    snapshot
        .entries
        .iter()
        .find(|entry| entry.unit == unit.as_str())
        .filter(|entry| {
            preconditions.matches(&entry.load_state, &entry.active_state, &entry.sub_state)
        })
        .ok_or_else(stale_observation)
}

impl Application {
    /// A session-bound, non-queuing inventory request. Successful publication
    /// replaces the complete actionable observation set for this host.
    pub async fn list_host_services(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
    ) -> Result<ServiceSnapshot, AppError> {
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
        let binding = AuthorityBinding {
            host_id,
            host_session_id,
            generation,
        };
        let sequence = self.service_observations.begin_request(host_id)?;
        let gate = {
            let mut gates = self.service_gates.lock().await;
            gates
                .entry(host_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let host_permit = gate.try_lock_owned().map_err(|_| busy())?;
        let global_permit = self.service_limit.try_acquire().map_err(|_| busy())?;
        drop(mutation);

        let mut snapshot = nexus_discovery::observe_services(
            transport.as_ref(),
            cancellation.clone(),
            host_id,
            host_session_id,
        )
        .await?;
        drop(global_permit);
        drop(host_permit);

        // Publication is protected by the same lifecycle gate used by
        // disconnect/delete/shutdown authority revocation.
        let _lifecycle = self
            .remote_operations
            .try_lifecycle_guard(host_id)
            .map_err(|_| busy())?;
        let data = slot.data.lock().await;
        if cancellation.is_cancelled()
            || transport.is_closed()
            || data.generation != generation
            || data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
            || data.view.host_session_id != Some(host_session_id)
            || !data
                .transport
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &transport))
        {
            return Err(cancelled());
        }
        self.service_observations
            .publish(binding, sequence, &mut snapshot)?;
        Ok(snapshot)
    }

    pub async fn plan_service_reset_failed(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        observation_id: ServiceObservationId,
    ) -> Result<ServiceResetFailedPlan, AppError> {
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
        let binding = AuthorityBinding {
            host_id,
            host_session_id,
            generation,
        };
        let observed = self.service_observations.resolve(observation_id, binding)?;
        let gate = {
            let mut gates = self.service_gates.lock().await;
            gates
                .entry(host_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let host_permit = gate.try_lock_owned().map_err(|_| busy())?;
        let global_permit = self.service_limit.try_acquire().map_err(|_| busy())?;
        let snapshot = nexus_discovery::observe_services(
            transport.as_ref(),
            cancellation.clone(),
            host_id,
            host_session_id,
        )
        .await?;
        exact_failed_service(&snapshot, &observed.unit, &observed.preconditions)?;
        drop(global_permit);
        drop(host_permit);

        let _lifecycle = self
            .remote_operations
            .try_lifecycle_guard(host_id)
            .map_err(|_| busy())?;
        let data = slot.data.lock().await;
        if cancellation.is_cancelled()
            || transport.is_closed()
            || data.generation != generation
            || data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
            || data.view.host_session_id != Some(host_session_id)
            || !data
                .transport
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &transport))
        {
            return Err(stale_session());
        }
        drop(data);
        // The original opaque observation must still be live and owned by the
        // same session when authority is published.
        let observed = self.service_observations.resolve(observation_id, binding)?;
        let receipt = self.remote_operations.plan::<SystemdResetFailed>(
            PlanDraft::new(binding, observed.unit.clone(), observed.preconditions, ()),
            binding,
        )?;
        Ok(ServiceResetFailedPlan {
            plan_id: receipt.id,
            host_id,
            host_session_id,
            unit: observed.unit.as_str().to_owned(),
            load_state: "loaded".into(),
            active_state: "failed".into(),
            sub_state: "failed".into(),
            risk: receipt.risk,
            expires_in_seconds: nexus_remote_operations::PLAN_TTL.as_secs(),
            effect: RESET_FAILED_EFFECT.into(),
        })
    }

    pub async fn discard_service_reset_failed(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        plan_id: RemoteOperationPlanId,
    ) -> Result<bool, AppError> {
        let _lifecycle = self
            .remote_operations
            .try_lifecycle_guard(host_id)
            .map_err(|_| busy())?;
        let slot = self.slot(host_id).await;
        let data = slot.data.lock().await;
        if data.view.state != ConnectionState::Connected
            || data.connection_id != Some(host_session_id)
            || data.view.host_session_id != Some(host_session_id)
        {
            return Err(stale_session());
        }
        let binding = AuthorityBinding {
            host_id,
            host_session_id,
            generation: data.generation,
        };
        drop(data);
        self.remote_operations.discard(plan_id, binding)
    }

    pub async fn execute_service_reset_failed(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        plan_id: RemoteOperationPlanId,
    ) -> Result<ServiceResetFailedResult, AppError> {
        let started = Instant::now();
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
        let binding = AuthorityBinding {
            host_id,
            host_session_id,
            generation,
        };
        let revalidator = ServiceResetRevalidator {
            application: self,
            slot: slot.clone(),
            transport: transport.clone(),
            cancellation: cancellation.clone(),
            binding,
        };
        let execution = self
            .remote_operations
            .execute::<SystemdResetFailed, _, _>(
                plan_id,
                binding,
                &revalidator,
                transport.as_ref(),
                cancellation,
            )
            .await?;
        let outcome = match execution.outcome {
            RemoteOperationOutcome::Success => ServiceResetFailedOutcome::Success,
            RemoteOperationOutcome::Failed => ServiceResetFailedOutcome::Failed,
            RemoteOperationOutcome::Cancelled => ServiceResetFailedOutcome::Cancelled,
            RemoteOperationOutcome::OutcomeUnknown => ServiceResetFailedOutcome::OutcomeUnknown,
        };
        let audit_outcome = match execution.outcome {
            RemoteOperationOutcome::Success => AuditOutcome::Success,
            RemoteOperationOutcome::Cancelled => AuditOutcome::Cancelled,
            RemoteOperationOutcome::Failed => AuditOutcome::Failed,
            RemoteOperationOutcome::OutcomeUnknown => AuditOutcome::OutcomeUnknown,
        };
        let audit_status = if self
            .record_with_risk(
                host_id,
                "service.reset_failed",
                OperationRisk::Moderate,
                audit_outcome,
                started,
            )
            .is_ok()
        {
            ServiceResetFailedAuditStatus::Persisted
        } else {
            ServiceResetFailedAuditStatus::Failed
        };

        // A single best-effort refresh is separate from the mutation truth.
        let snapshot = self.list_host_services(host_id, host_session_id).await.ok();
        let post_observation_status = if snapshot.is_some() {
            ServiceResetFailedPostObservationStatus::Refreshed
        } else {
            ServiceResetFailedPostObservationStatus::Unavailable
        };

        // Retain the explicit terminal mapping in this layer so adding a new
        // transport state cannot silently alter the public result contract.
        match execution.terminal {
            ExecutionTerminal::RevalidationFailed
            | ExecutionTerminal::CancelledBeforeDispatch
            | ExecutionTerminal::Transport(MutationTransportOutcome::NotDispatched(_))
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionConfirmed {
                ..
            })
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionUnknown(_)) => {}
        }
        Ok(ServiceResetFailedResult {
            outcome,
            audit_status,
            post_observation_status,
            snapshot,
        })
    }
}

struct ServiceResetRevalidator<'a> {
    application: &'a Application,
    slot: Arc<crate::sessions::SessionSlot>,
    transport: Arc<dyn crate::ConnectedTransport>,
    cancellation: CancellationToken,
    binding: AuthorityBinding,
}

#[async_trait::async_trait]
impl AuthorityRevalidator<SystemdResetFailed> for ServiceResetRevalidator<'_> {
    type DispatchGuard = OwnedMutexGuard<()>;

    async fn revalidate(
        &self,
        authority: &ConsumedAuthority<SystemdResetFailed>,
    ) -> Result<Self::DispatchGuard, AppError> {
        if authority.binding() != self.binding
            || authority.risk() != OperationRisk::Moderate
            || self.cancellation.is_cancelled()
        {
            return Err(stale_observation());
        }
        let gate = {
            let gates = self.application.service_gates.lock().await;
            gates.get(&self.binding.host_id).cloned().ok_or_else(busy)?
        };
        let host_guard = gate.try_lock_owned().map_err(|_| busy())?;
        let global_permit = self
            .application
            .service_limit
            .try_acquire()
            .map_err(|_| busy())?;
        let snapshot = nexus_discovery::observe_services(
            self.transport.as_ref(),
            self.cancellation.clone(),
            self.binding.host_id,
            self.binding.host_session_id,
        )
        .await?;
        exact_failed_service(&snapshot, authority.target(), authority.preconditions())?;
        let data = self.slot.data.lock().await;
        if self.cancellation.is_cancelled()
            || self.transport.is_closed()
            || data.generation != self.binding.generation
            || data.view.state != ConnectionState::Connected
            || data.connection_id != Some(self.binding.host_session_id)
            || data.view.host_session_id != Some(self.binding.host_session_id)
            || !data
                .transport
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.transport))
        {
            return Err(stale_session());
        }
        drop(data);
        drop(global_permit);
        Ok(host_guard)
    }
}
