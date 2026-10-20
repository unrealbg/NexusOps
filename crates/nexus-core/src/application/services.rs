use super::*;
use nexus_remote_operations::{
    AuthorityBinding, AuthorityRevalidator, ConsumedAuthority, ExecutionTerminal,
    MutationTransportOutcome, PlanDraft, RemoteOperationOutcome, SystemdReload,
    SystemdReloadPreconditions, SystemdResetFailed, SystemdResetFailedPreconditions,
    SystemdServiceUnitName, SystemdStart, SystemdStartPreconditions, SystemdTryRestart,
    SystemdTryRestartPreconditions,
};
use tokio::sync::OwnedMutexGuard;

const RESET_FAILED_EFFECT: &str = "Clears the selected service's failed state and rate-limit/restart counters. It does not intentionally start or stop the service, but it can affect later service behavior.";
const TRY_RESTART_EFFECT: &str = "Restarts the selected running service, which may be temporarily unavailable. Systemd may affect dependency-related jobs according to the unit configuration. Successful command completion does not guarantee application-level health. If the service becomes inactive before command execution, try-restart may perform no restart. There is no rollback.";
const RELOAD_EFFECT: &str = "Requests the selected running service to reload using its systemd-defined reload mechanism. The service may apply configuration immediately and dependent traffic or behavior may change. CanReload only indicates that systemd exposes reload support; it does not guarantee application-level safety or health. Successful command completion does not prove the application is healthy. There is no rollback.";
const START_EFFECT: &str = "Requests systemd to start the selected inactive service. Starting the service executes its configured start behavior and may activate dependencies, bind network listeners, process work, or otherwise change host behavior. CanStart only indicates that systemd exposes start support; it does not guarantee permission, configuration validity, application health, or that the service will remain running. There is no automatic rollback.";

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

fn exact_running_service<'a>(
    snapshot: &'a ServiceSnapshot,
    unit: &SystemdServiceUnitName,
    preconditions: &SystemdTryRestartPreconditions,
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

fn exact_reloadable_service<'a>(
    snapshot: &'a ServiceSnapshot,
    unit: &SystemdServiceUnitName,
    preconditions: &SystemdReloadPreconditions,
) -> Result<&'a ServiceEntry, AppError> {
    snapshot
        .entries
        .iter()
        .find(|entry| entry.unit == unit.as_str())
        .filter(|entry| {
            preconditions.matches(
                &entry.load_state,
                &entry.active_state,
                &entry.sub_state,
                entry.can_reload,
            )
        })
        .ok_or_else(stale_observation)
}

fn exact_startable_service<'a>(
    snapshot: &'a ServiceSnapshot,
    unit: &SystemdServiceUnitName,
    preconditions: &SystemdStartPreconditions,
) -> Result<&'a ServiceEntry, AppError> {
    snapshot
        .entries
        .iter()
        .find(|entry| entry.unit == unit.as_str())
        .filter(|entry| {
            preconditions.matches(
                &entry.load_state,
                &entry.active_state,
                &entry.sub_state,
                entry.can_start,
            )
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
        let inspection_sequence = self.stop_impact_inspections.begin_request(host_id)?;
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
        self.stop_impact_inspections
            .publish(binding, inspection_sequence, &mut snapshot)?;
        Ok(snapshot)
    }

    /// Resolves a renderer-held opaque inspection identity to its native-owned
    /// service target, then performs only the bounded read-only assessment.
    pub async fn assess_service_stop_impact(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        inspection_id: SystemdStopImpactInspectionId,
    ) -> Result<SystemdStopImpactAssessment, AppError> {
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
        let root_unit = self
            .stop_impact_inspections
            .resolve(inspection_id, binding)?;
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

        let assessment = nexus_discovery::assess_systemd_stop_impact(
            transport.as_ref(),
            cancellation.clone(),
            host_id,
            host_session_id,
            root_unit,
        )
        .await?;
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
            return Err(cancelled());
        }
        drop(data);
        // Re-resolve after the asynchronous work so refresh/reconnect/edit
        // invalidation cannot publish a stale assessment.
        self.stop_impact_inspections
            .resolve(inspection_id, binding)?;
        Ok(assessment)
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
        let observed = self
            .service_observations
            .resolve_reset_failed(observation_id, binding)?;
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
        let observed = self
            .service_observations
            .resolve_reset_failed(observation_id, binding)?;
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
        self.remote_operations
            .discard::<SystemdResetFailed>(plan_id, binding)
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

    pub async fn plan_service_try_restart(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        observation_id: ServiceObservationId,
    ) -> Result<ServiceTryRestartPlan, AppError> {
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
        let observed = self
            .service_observations
            .resolve_try_restart(observation_id, binding)?;
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
        exact_running_service(&snapshot, &observed.unit, &observed.preconditions)?;
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
        let observed = self
            .service_observations
            .resolve_try_restart(observation_id, binding)?;
        let receipt = self.remote_operations.plan::<SystemdTryRestart>(
            PlanDraft::new(binding, observed.unit.clone(), observed.preconditions, ()),
            binding,
        )?;
        Ok(ServiceTryRestartPlan {
            plan_id: receipt.id,
            host_id,
            host_session_id,
            unit: observed.unit.as_str().to_owned(),
            load_state: "loaded".into(),
            active_state: "active".into(),
            sub_state: "running".into(),
            risk: receipt.risk,
            expires_in_seconds: nexus_remote_operations::PLAN_TTL.as_secs(),
            effect: TRY_RESTART_EFFECT.into(),
        })
    }

    pub async fn discard_service_try_restart(
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
        self.remote_operations
            .discard::<SystemdTryRestart>(plan_id, binding)
    }

    pub async fn execute_service_try_restart(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        plan_id: RemoteOperationPlanId,
    ) -> Result<ServiceTryRestartResult, AppError> {
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
        let revalidator = ServiceTryRestartRevalidator {
            application: self,
            slot: slot.clone(),
            transport: transport.clone(),
            cancellation: cancellation.clone(),
            binding,
        };
        let execution = self
            .remote_operations
            .execute::<SystemdTryRestart, _, _>(
                plan_id,
                binding,
                &revalidator,
                transport.as_ref(),
                cancellation,
            )
            .await?;
        let outcome = match execution.outcome {
            RemoteOperationOutcome::Success => ServiceTryRestartOutcome::Success,
            RemoteOperationOutcome::Failed => ServiceTryRestartOutcome::Failed,
            RemoteOperationOutcome::Cancelled => ServiceTryRestartOutcome::Cancelled,
            RemoteOperationOutcome::OutcomeUnknown => ServiceTryRestartOutcome::OutcomeUnknown,
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
                "service.try_restart",
                OperationRisk::High,
                audit_outcome,
                started,
            )
            .is_ok()
        {
            ServiceTryRestartAuditStatus::Persisted
        } else {
            ServiceTryRestartAuditStatus::Failed
        };
        let snapshot = self.list_host_services(host_id, host_session_id).await.ok();
        let post_observation_status = if snapshot.is_some() {
            ServiceTryRestartPostObservationStatus::Refreshed
        } else {
            ServiceTryRestartPostObservationStatus::Unavailable
        };
        match execution.terminal {
            ExecutionTerminal::RevalidationFailed
            | ExecutionTerminal::CancelledBeforeDispatch
            | ExecutionTerminal::Transport(MutationTransportOutcome::NotDispatched(_))
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionConfirmed {
                ..
            })
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionUnknown(_)) => {}
        }
        Ok(ServiceTryRestartResult {
            outcome,
            audit_status,
            post_observation_status,
            snapshot,
        })
    }
    pub async fn plan_service_reload(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        observation_id: ServiceObservationId,
    ) -> Result<ServiceReloadPlan, AppError> {
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
        let observed = self
            .service_observations
            .resolve_reload(observation_id, binding)?;
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
        exact_reloadable_service(&snapshot, &observed.unit, &observed.preconditions)?;
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
        let observed = self
            .service_observations
            .resolve_reload(observation_id, binding)?;
        let receipt = self.remote_operations.plan::<SystemdReload>(
            PlanDraft::new(binding, observed.unit.clone(), observed.preconditions, ()),
            binding,
        )?;
        Ok(ServiceReloadPlan {
            plan_id: receipt.id,
            host_id,
            host_session_id,
            unit: observed.unit.as_str().to_owned(),
            load_state: "loaded".into(),
            active_state: "active".into(),
            sub_state: "running".into(),
            can_reload: true,
            risk: receipt.risk,
            expires_in_seconds: nexus_remote_operations::PLAN_TTL.as_secs(),
            effect: RELOAD_EFFECT.into(),
        })
    }

    pub async fn discard_service_reload(
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
        self.remote_operations
            .discard::<SystemdReload>(plan_id, binding)
    }

    pub async fn execute_service_reload(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        plan_id: RemoteOperationPlanId,
    ) -> Result<ServiceReloadResult, AppError> {
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
        let revalidator = ServiceReloadRevalidator {
            application: self,
            slot: slot.clone(),
            transport: transport.clone(),
            cancellation: cancellation.clone(),
            binding,
        };
        let execution = self
            .remote_operations
            .execute::<SystemdReload, _, _>(
                plan_id,
                binding,
                &revalidator,
                transport.as_ref(),
                cancellation,
            )
            .await?;
        let outcome = match execution.outcome {
            RemoteOperationOutcome::Success => ServiceReloadOutcome::Success,
            RemoteOperationOutcome::Failed => ServiceReloadOutcome::Failed,
            RemoteOperationOutcome::Cancelled => ServiceReloadOutcome::Cancelled,
            RemoteOperationOutcome::OutcomeUnknown => ServiceReloadOutcome::OutcomeUnknown,
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
                "service.reload",
                OperationRisk::High,
                audit_outcome,
                started,
            )
            .is_ok()
        {
            ServiceReloadAuditStatus::Persisted
        } else {
            ServiceReloadAuditStatus::Failed
        };
        let snapshot = self.list_host_services(host_id, host_session_id).await.ok();
        let post_observation_status = if snapshot.is_some() {
            ServiceReloadPostObservationStatus::Refreshed
        } else {
            ServiceReloadPostObservationStatus::Unavailable
        };
        match execution.terminal {
            ExecutionTerminal::RevalidationFailed
            | ExecutionTerminal::CancelledBeforeDispatch
            | ExecutionTerminal::Transport(MutationTransportOutcome::NotDispatched(_))
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionConfirmed {
                ..
            })
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionUnknown(_)) => {}
        }
        Ok(ServiceReloadResult {
            outcome,
            audit_status,
            post_observation_status,
            snapshot,
        })
    }
    pub async fn plan_service_start(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        observation_id: ServiceObservationId,
    ) -> Result<ServiceStartPlan, AppError> {
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
        let observed = self
            .service_observations
            .resolve_start(observation_id, binding)?;
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
        exact_startable_service(&snapshot, &observed.unit, &observed.preconditions)?;
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
        let observed = self
            .service_observations
            .resolve_start(observation_id, binding)?;
        let receipt = self.remote_operations.plan::<SystemdStart>(
            PlanDraft::new(binding, observed.unit.clone(), observed.preconditions, ()),
            binding,
        )?;
        Ok(ServiceStartPlan {
            plan_id: receipt.id,
            host_id,
            host_session_id,
            unit: observed.unit.as_str().to_owned(),
            load_state: "loaded".into(),
            active_state: "inactive".into(),
            sub_state: "dead".into(),
            can_start: true,
            risk: receipt.risk,
            expires_in_seconds: nexus_remote_operations::PLAN_TTL.as_secs(),
            effect: START_EFFECT.into(),
        })
    }

    pub async fn discard_service_start(
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
        self.remote_operations
            .discard::<SystemdStart>(plan_id, binding)
    }

    pub async fn execute_service_start(
        &self,
        host_id: HostId,
        host_session_id: HostSessionId,
        plan_id: RemoteOperationPlanId,
    ) -> Result<ServiceStartResult, AppError> {
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
        let revalidator = ServiceStartRevalidator {
            application: self,
            slot: slot.clone(),
            transport: transport.clone(),
            cancellation: cancellation.clone(),
            binding,
        };
        let execution = self
            .remote_operations
            .execute::<SystemdStart, _, _>(
                plan_id,
                binding,
                &revalidator,
                transport.as_ref(),
                cancellation,
            )
            .await?;
        let outcome = match execution.outcome {
            RemoteOperationOutcome::Success => ServiceStartOutcome::Success,
            RemoteOperationOutcome::Failed => ServiceStartOutcome::Failed,
            RemoteOperationOutcome::Cancelled => ServiceStartOutcome::Cancelled,
            RemoteOperationOutcome::OutcomeUnknown => ServiceStartOutcome::OutcomeUnknown,
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
                "service.start",
                OperationRisk::High,
                audit_outcome,
                started,
            )
            .is_ok()
        {
            ServiceStartAuditStatus::Persisted
        } else {
            ServiceStartAuditStatus::Failed
        };
        let snapshot = self.list_host_services(host_id, host_session_id).await.ok();
        let post_observation_status = if snapshot.is_some() {
            ServiceStartPostObservationStatus::Refreshed
        } else {
            ServiceStartPostObservationStatus::Unavailable
        };
        match execution.terminal {
            ExecutionTerminal::RevalidationFailed
            | ExecutionTerminal::CancelledBeforeDispatch
            | ExecutionTerminal::Transport(MutationTransportOutcome::NotDispatched(_))
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionConfirmed {
                ..
            })
            | ExecutionTerminal::Transport(MutationTransportOutcome::CompletionUnknown(_)) => {}
        }
        Ok(ServiceStartResult {
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

struct ServiceTryRestartRevalidator<'a> {
    application: &'a Application,
    slot: Arc<crate::sessions::SessionSlot>,
    transport: Arc<dyn crate::ConnectedTransport>,
    cancellation: CancellationToken,
    binding: AuthorityBinding,
}

#[async_trait::async_trait]
impl AuthorityRevalidator<SystemdTryRestart> for ServiceTryRestartRevalidator<'_> {
    type DispatchGuard = OwnedMutexGuard<()>;

    async fn revalidate(
        &self,
        authority: &ConsumedAuthority<SystemdTryRestart>,
    ) -> Result<Self::DispatchGuard, AppError> {
        if authority.binding() != self.binding
            || authority.risk() != OperationRisk::High
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
        exact_running_service(&snapshot, authority.target(), authority.preconditions())?;
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

struct ServiceReloadRevalidator<'a> {
    application: &'a Application,
    slot: Arc<crate::sessions::SessionSlot>,
    transport: Arc<dyn crate::ConnectedTransport>,
    cancellation: CancellationToken,
    binding: AuthorityBinding,
}

#[async_trait::async_trait]
impl AuthorityRevalidator<SystemdReload> for ServiceReloadRevalidator<'_> {
    type DispatchGuard = OwnedMutexGuard<()>;

    async fn revalidate(
        &self,
        authority: &ConsumedAuthority<SystemdReload>,
    ) -> Result<Self::DispatchGuard, AppError> {
        if authority.binding() != self.binding
            || authority.risk() != OperationRisk::High
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
        exact_reloadable_service(&snapshot, authority.target(), authority.preconditions())?;
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

struct ServiceStartRevalidator<'a> {
    application: &'a Application,
    slot: Arc<crate::sessions::SessionSlot>,
    transport: Arc<dyn crate::ConnectedTransport>,
    cancellation: CancellationToken,
    binding: AuthorityBinding,
}

#[async_trait::async_trait]
impl AuthorityRevalidator<SystemdStart> for ServiceStartRevalidator<'_> {
    type DispatchGuard = OwnedMutexGuard<()>;

    async fn revalidate(
        &self,
        authority: &ConsumedAuthority<SystemdStart>,
    ) -> Result<Self::DispatchGuard, AppError> {
        if authority.binding() != self.binding
            || authority.risk() != OperationRisk::High
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
        exact_startable_service(&snapshot, authority.target(), authority.preconditions())?;
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
