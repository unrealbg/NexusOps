import { useEffect, useMemo, useRef, useState } from 'react';
import type {
  Host,
  ServiceObservationId,
  ServiceResetFailedPlan,
  ServiceResetFailedResult,
  ServiceSnapshot,
  ServiceTryRestartPlan,
  ServiceTryRestartResult,
} from '@nexusops/protocol';
import { Button, Modal, Notice, Spinner } from '@nexusops/ui';
import { servicesApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

type OwnedPlan =
  | { owner: string; kind: 'resetFailed'; plan: ServiceResetFailedPlan }
  | { owner: string; kind: 'tryRestart'; plan: ServiceTryRestartPlan };
type OwnedResult =
  | { owner: string; kind: 'resetFailed'; value: ServiceResetFailedResult }
  | { owner: string; kind: 'tryRestart'; value: ServiceTryRestartResult };

export function ServicesWorkspace({ host }: { host: Host }) {
  const sessionQuery = useHostSession(host.id);
  const session = sessionQuery.data;
  const connected = session?.state === 'connected';
  const sessionId = connected ? session.hostSessionId : null;
  const owner = `${host.id}:${sessionId ?? ''}`;
  const [snapshot, setSnapshot] = useState<ServiceSnapshot | null>(null);
  const [error, setError] = useState<{ owner: string; failedRefresh: boolean; operation?: boolean } | null>(null);
  const [refreshingOwner, setRefreshingOwner] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [ownedPlan, setOwnedPlan] = useState<OwnedPlan | null>(null);
  const planRef = useRef<OwnedPlan | null>(null);
  const [planningObservation, setPlanningObservation] = useState<{ owner: string; id: ServiceObservationId; kind: OwnedPlan['kind'] } | null>(null);
  const [independentlyVerified, setIndependentlyVerified] = useState(false);
  const [executingOwner, setExecutingOwner] = useState<string | null>(null);
  const [result, setResult] = useState<OwnedResult | null>(null);
  const generation = useRef(0);
  const inFlight = useRef(false);
  const actionInFlight = useRef(false);

  function publishPlan(value: OwnedPlan | null) {
    planRef.current = value;
    setOwnedPlan(value);
    setIndependentlyVerified(false);
  }

  useEffect(() => {
    const current = ++generation.current;
    inFlight.current = false;
    actionInFlight.current = false;
    const cleanup = () => {
      generation.current += 1;
      const currentPlan = planRef.current;
      if (currentPlan?.owner === owner) {
        planRef.current = null;
        setOwnedPlan(null);
        setIndependentlyVerified(false);
        const discard = currentPlan.kind === 'resetFailed'
          ? servicesApi.discardResetFailed
          : servicesApi.discardTryRestart;
        void discard(
          currentPlan.plan.hostId,
          currentPlan.plan.hostSessionId,
          currentPlan.plan.planId,
        ).catch(() => undefined);
      }
    };
    if (!connected || !sessionId) return cleanup;
    inFlight.current = true;
    void servicesApi.list(host.id, sessionId).then(
      (value) => {
        if (generation.current !== current) return;
        if (value.hostId !== host.id || value.hostSessionId !== sessionId) {
          setError({ owner, failedRefresh: false });
          return;
        }
        setSnapshot(value);
      },
      () => {
        if (generation.current === current) setError({ owner, failedRefresh: false });
      },
    ).finally(() => {
      if (generation.current === current) inFlight.current = false;
    });
    return cleanup;
  }, [connected, host.id, owner, sessionId]);

  const validSnapshot = connected && snapshot?.hostId === host.id && snapshot.hostSessionId === sessionId
    ? snapshot : null;
  const visibleError = error?.owner === owner ? error : null;
  const visibleResult = result?.owner === owner ? result : null;
  const livePlan = ownedPlan?.owner === owner ? ownedPlan.plan : null;
  const visiblePlanningObservation = planningObservation?.owner === owner
    ? planningObservation : null;
  const executing = executingOwner === owner;
  const pending = refreshingOwner === owner || (connected && !validSnapshot && !visibleError);
  const rows = useMemo(() => {
    if (!validSnapshot) return [];
    const term = filter.trim().toLocaleLowerCase();
    if (!term) return validSnapshot.entries;
    return validSnapshot.entries.filter((entry) =>
      [entry.unit, entry.activeState, entry.subState, entry.description]
        .some((field) => field.toLocaleLowerCase().includes(term)),
    );
  }, [filter, validSnapshot]);

  function refresh() {
    if (!connected || !sessionId || inFlight.current || actionInFlight.current) return;
    const current = generation.current;
    const hadSnapshot = validSnapshot !== null;
    inFlight.current = true;
    setRefreshingOwner(owner);
    setError(null);
    setResult(null);
    void servicesApi.list(host.id, sessionId).then(
      (value) => {
        if (generation.current !== current) return;
        if (value.hostId !== host.id || value.hostSessionId !== sessionId) {
          setError({ owner, failedRefresh: hadSnapshot });
          return;
        }
        setSnapshot(value);
      },
      () => {
        if (generation.current === current) setError({ owner, failedRefresh: hadSnapshot });
      },
    ).finally(() => {
      if (generation.current === current) {
        inFlight.current = false;
        setRefreshingOwner(null);
      }
    });
  }

  function planResetFailed(observationId: ServiceObservationId) {
    if (!connected || !sessionId || actionInFlight.current || planRef.current) return;
    actionInFlight.current = true;
    setPlanningObservation({ owner, id: observationId, kind: 'resetFailed' });
    setError(null);
    setResult(null);
    const requestOwner = owner;
    const requestGeneration = generation.current;
    void servicesApi.planResetFailed(host.id, sessionId, observationId).then(
      (plan) => {
        if (
          generation.current !== requestGeneration
          || plan.hostId !== host.id
          || plan.hostSessionId !== sessionId
        ) {
          void servicesApi.discardResetFailed(plan.hostId, plan.hostSessionId, plan.planId)
            .catch(() => undefined);
          return;
        }
        publishPlan({ owner: requestOwner, kind: 'resetFailed', plan });
      },
      () => {
        if (generation.current === requestGeneration) {
          setError({ owner: requestOwner, failedRefresh: true, operation: true });
        }
      },
    ).finally(() => {
      if (generation.current === requestGeneration) {
        actionInFlight.current = false;
        setPlanningObservation(null);
      }
    });
  }

  function planTryRestart(observationId: ServiceObservationId) {
    if (!connected || !sessionId || actionInFlight.current || planRef.current) return;
    actionInFlight.current = true;
    setPlanningObservation({ owner, id: observationId, kind: 'tryRestart' });
    setError(null);
    setResult(null);
    const requestOwner = owner;
    const requestGeneration = generation.current;
    void servicesApi.planTryRestart(host.id, sessionId, observationId).then(
      (plan) => {
        if (
          generation.current !== requestGeneration
          || plan.hostId !== host.id
          || plan.hostSessionId !== sessionId
        ) {
          void servicesApi.discardTryRestart(plan.hostId, plan.hostSessionId, plan.planId)
            .catch(() => undefined);
          return;
        }
        publishPlan({ owner: requestOwner, kind: 'tryRestart', plan });
      },
      () => {
        if (generation.current === requestGeneration) {
          setError({ owner: requestOwner, failedRefresh: true, operation: true });
        }
      },
    ).finally(() => {
      if (generation.current === requestGeneration) {
        actionInFlight.current = false;
        setPlanningObservation(null);
      }
    });
  }

  function cancelPlan() {
    if (actionInFlight.current) return;
    const current = planRef.current;
    if (!current) return;
    actionInFlight.current = true;
    publishPlan(null);
    const discard = current.kind === 'resetFailed'
      ? servicesApi.discardResetFailed
      : servicesApi.discardTryRestart;
    void discard(
      current.plan.hostId,
      current.plan.hostSessionId,
      current.plan.planId,
    ).catch(() => undefined).finally(() => {
      actionInFlight.current = false;
    });
  }

  function executePlan() {
    const current = planRef.current;
    if (!current || !independentlyVerified || actionInFlight.current) return;
    actionInFlight.current = true;
    setExecutingOwner(current.owner);
    publishPlan(null);
    const requestGeneration = generation.current;
    const execute = current.kind === 'resetFailed'
      ? servicesApi.executeResetFailed
      : servicesApi.executeTryRestart;
    void execute(
      current.plan.hostId,
      current.plan.hostSessionId,
      current.plan.planId,
    ).then(
      (value) => {
        if (generation.current !== requestGeneration) return;
        if (value.snapshot
          && value.snapshot.hostId === current.plan.hostId
          && value.snapshot.hostSessionId === current.plan.hostSessionId) {
          setSnapshot(value.snapshot);
        }
        setResult({ owner: current.owner, kind: current.kind, value } as OwnedResult);
      },
      () => {
        if (generation.current === requestGeneration) {
          setError({ owner: current.owner, failedRefresh: true, operation: true });
        }
      },
    ).finally(() => {
      if (generation.current === requestGeneration) {
        actionInFlight.current = false;
        setExecutingOwner(null);
      }
    });
  }

  return (
    <section aria-labelledby="services-title">
      <header className="page-heading">
        <div><div className="eyebrow">SYSTEMD INVENTORY</div><h1 id="services-title">Services</h1><p>{host.displayName} · {connected ? 'Connected' : 'Disconnected'}</p></div>
        <Button onClick={refresh} disabled={!connected || !sessionId || pending || executing}>Refresh</Button>
      </header>
      {!connected ? <p>Connect this host to inspect system services.</p> : (
        <>
          {pending && !validSnapshot && <Spinner label="Reading system services…" />}
          {executing && <Spinner label={livePlan?.activeState === 'active' ? 'Restarting active service…' : 'Applying service operation…'} />}
          {visibleError && (
            <Notice>{visibleError.operation
              ? 'The operation could not be completed. Refresh services before trying again.'
              : visibleError.failedRefresh && validSnapshot
              ? 'Refresh failed — showing the last successful snapshot.'
              : 'Service inventory is unavailable on this host.'}</Notice>
          )}
          {visibleResult && <Notice tone={visibleResult.value.outcome === 'success' ? 'neutral' : 'warning'}>
            {visibleResult.value.outcome === 'success' && visibleResult.kind === 'resetFailed' && 'The failed-state marker was cleared.'}
            {visibleResult.value.outcome === 'success' && visibleResult.kind === 'tryRestart' && 'The systemd try-restart operation completed. The refreshed service state is shown below.'}
            {visibleResult.value.outcome === 'failed' && 'The service operation failed.'}
            {visibleResult.value.outcome === 'cancelled' && 'The service operation was cancelled before dispatch.'}
            {visibleResult.value.outcome === 'outcomeUnknown' && 'The connection ended before the remote outcome could be confirmed. Refresh before taking another action.'}
            {visibleResult.value.auditStatus === 'failed' && ' The local audit record could not be persisted.'}
            {visibleResult.value.postObservationStatus === 'unavailable' && ' The post-operation service refresh was unavailable.'}
          </Notice>}
          {validSnapshot && (
            <>
              <p>Observed <time dateTime={validSnapshot.observedAt}>{validSnapshot.observedAt}</time> · {rows.length} of {validSnapshot.entries.length} services</p>
              <label htmlFor="service-filter">Filter services</label>
              <input id="service-filter" type="search" value={filter} onChange={(event) => setFilter(event.target.value)} />
              {validSnapshot.entries.length === 0 ? <p>No loaded system services were returned.</p>
                : rows.length === 0 ? <p>No services match this filter.</p>
                : <div className="files-table-wrap"><table className="files-table"><thead><tr><th scope="col">Service</th><th scope="col">Load</th><th scope="col">Active</th><th scope="col">Sub</th><th scope="col">Description</th><th scope="col">Action</th></tr></thead><tbody>
                    {rows.map((entry) => <tr key={entry.unit}><td>{entry.unit}</td><td>{entry.loadState}</td><td>{entry.activeState}</td><td>{entry.subState}</td><td>{entry.description}</td><td>{entry.resetFailedObservationId
                      ? <Button disabled={visiblePlanningObservation !== null || executing || livePlan !== null} onClick={() => planResetFailed(entry.resetFailedObservationId!)}>{visiblePlanningObservation?.kind === 'resetFailed' && visiblePlanningObservation.id === entry.resetFailedObservationId ? 'Preparing…' : 'Reset failed state'}</Button>
                      : entry.tryRestartObservationId
                      ? <Button variant="danger" disabled={visiblePlanningObservation !== null || executing || livePlan !== null} onClick={() => planTryRestart(entry.tryRestartObservationId!)}>{visiblePlanningObservation?.kind === 'tryRestart' && visiblePlanningObservation.id === entry.tryRestartObservationId ? 'Preparing…' : 'Restart active service'}</Button>
                      : '—'}</td></tr>)}
                  </tbody></table></div>}
            </>
          )}
          {livePlan && <Modal title="Approve systemd operation" onClose={cancelPlan}>
            <p className="dialog-description">Host: <strong>{host.displayName}</strong></p>
            <dl className="properties"><dt>Service</dt><dd><code>{livePlan.unit}</code></dd><dt>Observed state</dt><dd>{livePlan.loadState} / {livePlan.activeState} / {livePlan.subState}</dd><dt>Risk</dt><dd>{livePlan.risk === 'moderate' ? 'Moderate' : livePlan.risk === 'high' ? 'High' : livePlan.risk}</dd><dt>Effect</dt><dd>{livePlan.effect}</dd></dl>
            <Notice tone="warning">This one-time approval expires in {livePlan.expiresInSeconds.toString()} seconds. {ownedPlan?.kind === 'tryRestart' ? 'The running service and dependent traffic may be temporarily interrupted. There is no rollback.' : 'It clears failed, rate-limit, and restart counters and may affect later service behavior. It does not intentionally start or stop the service.'}</Notice>
            <label><input data-initial-focus type="checkbox" checked={independentlyVerified} onChange={(event) => setIndependentlyVerified(event.target.checked)} /> {ownedPlan?.kind === 'tryRestart' ? 'I verified this exact running service and understand that restarting it may temporarily interrupt the service and dependent traffic.' : 'I independently verified this exact service target and want to clear only its failed-state marker.'}</label>
            <div className="modal-actions"><Button onClick={cancelPlan}>Cancel</Button><Button variant={ownedPlan?.kind === 'tryRestart' ? 'danger' : 'primary'} disabled={!independentlyVerified} onClick={executePlan}>{ownedPlan?.kind === 'tryRestart' ? 'Restart active service' : 'Reset failed state'}</Button></div>
          </Modal>}
        </>
      )}
    </section>
  );
}
