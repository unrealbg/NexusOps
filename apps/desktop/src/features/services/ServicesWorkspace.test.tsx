import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Host, ServiceSnapshot, SystemdStopImpactAssessment } from '@nexusops/protocol';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  state: 'disconnected',
  sessionId: null as string | null,
  list: vi.fn(),
  assessStopImpact: vi.fn(),
  planResetFailed: vi.fn(),
  discardResetFailed: vi.fn(),
  executeResetFailed: vi.fn(),
  planTryRestart: vi.fn(),
  discardTryRestart: vi.fn(),
  executeTryRestart: vi.fn(),
  planReload: vi.fn(),
  discardReload: vi.fn(),
  executeReload: vi.fn(),
  planStart: vi.fn(),
  discardStart: vi.fn(),
  executeStart: vi.fn(),
}));
vi.mock('../../api/queries', () => ({
  useHostSession: () => ({ data: { state: mocks.state, hostSessionId: mocks.sessionId } }),
}));
vi.mock('../../api/client', () => ({ servicesApi: {
  list: mocks.list,
  assessStopImpact: mocks.assessStopImpact,
  planResetFailed: mocks.planResetFailed,
  discardResetFailed: mocks.discardResetFailed,
  executeResetFailed: mocks.executeResetFailed,
  planTryRestart: mocks.planTryRestart,
  discardTryRestart: mocks.discardTryRestart,
  executeTryRestart: mocks.executeTryRestart,
  planReload: mocks.planReload,
  discardReload: mocks.discardReload,
  executeReload: mocks.executeReload,
  planStart: mocks.planStart,
  discardStart: mocks.discardStart,
  executeStart: mocks.executeStart,
} }));

import { ServicesWorkspace } from './ServicesWorkspace';

const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const host = { id: 'host-a', displayName: 'Alpha' } as Host;
const otherHost = { id: 'host-b', displayName: 'Beta' } as Host;
function snapshot(hostId = host.id, hostSessionId = A): ServiceSnapshot {
  return {
    hostId, hostSessionId, observedAt: '2026-09-26T12:00:00Z',
    entries: [
      { unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running', description: 'OpenSSH server', canStart: false, canReload: false, resetFailedObservationId: null , tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null},
      { unit: 'future.service', loadState: 'loaded', activeState: 'repairing', subState: 'unknown', description: '<script>bad</script> text', canStart: false, canReload: false, resetFailedObservationId: null , tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null},
    ],
  };
}
function stopImpactAssessment(hostSessionId = A): SystemdStopImpactAssessment {
  return {
    hostId: host.id, hostSessionId, rootUnit: 'ssh.service', observedAt: '2026-10-10T00:00:00Z',
    completeness: 'complete', uncertainty: 'directOnly', rootConsistent: true,
    units: [], edges: [], conditionalConsequences: [], conditionalDiagnostics: [], warnings: [], limitations: [],
    accounting: {
      observedUnitRecords: 1, observedRelationshipReferences: 0, observedCandidateReferences: 0,
      retainedUnits: 1, retainedEdges: 0, retainedCandidates: 0, retainedDiagnostics: 0,
      omittedKnownUnits: 0, omittedKnownEdges: 0, omittedKnownCandidates: 0,
      omittedKnownDiagnostics: 0, unresolvedFrontierReferences: 0,
      actualSshQueries: 2,
    },
  };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  mocks.state = 'disconnected';
  mocks.sessionId = null;
  mocks.list.mockReset();
  mocks.assessStopImpact.mockReset();
  mocks.planResetFailed.mockReset();
  mocks.discardResetFailed.mockReset().mockResolvedValue(true);
  mocks.executeResetFailed.mockReset();
  mocks.planTryRestart.mockReset();
  mocks.discardTryRestart.mockReset().mockResolvedValue(true);
  mocks.executeTryRestart.mockReset();
  mocks.planReload.mockReset();
  mocks.discardReload.mockReset().mockResolvedValue(true);
  mocks.executeReload.mockReset();
  mocks.planStart.mockReset();
  mocks.discardStart.mockReset().mockResolvedValue(true);
  mocks.executeStart.mockReset();
});

describe('session-bound Services workspace', () => {
  it('does not request disconnected hosts', () => {
    render(<ServicesWorkspace host={host} />);
    expect(screen.getByText('Connect this host to inspect system services.')).toBeInTheDocument();
    expect(mocks.list).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
  });

  it('loads once, filters locally, renders text only and refreshes exactly once while pending', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const refresh = deferred<ServiceSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(refresh.promise);
    render(<ServicesWorkspace host={host} />);
    expect(await screen.findByText('ssh.service')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(screen.getByText('<script>bad</script> text')).toBeInTheDocument();
    expect(document.querySelector('script')).toBeNull();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Filter services' }), 'repairing');
    expect(screen.queryByText('ssh.service')).not.toBeInTheDocument();
    expect(screen.getByText('future.service')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(1);
    const button = screen.getByRole('button', { name: 'Refresh' });
    await userEvent.click(button);
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    await act(async () => { refresh.resolve(snapshot()); });
    await waitFor(() => expect(button).toBeEnabled());
  });

  it('requests one opaque read-only assessment and renders bounded diagnostic truth', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const value = snapshot();
    value.entries[0]!.stopImpactInspectionId = 'inspection-a';
    mocks.list.mockResolvedValueOnce(value);
    mocks.assessStopImpact.mockResolvedValueOnce({
      hostId: host.id,
      hostSessionId: A,
      rootUnit: 'ssh.service',
      observedAt: '2026-10-09T00:00:00Z',
      completeness: 'partial',
      uncertainty: 'unknownImpact',
      rootConsistent: false,
      units: [{
        canonicalUnit: 'ssh.service', loadState: 'loaded', activeState: 'active',
        subState: 'running', canStop: true, refuseManualStop: false,
        stopWhenUnneeded: false, hasPendingJob: false, depth: 0, provenance: 'direct',
      }],
      edges: [{
        source: 'ssh.service', target: 'worker.service', relationship: 'requiredBy',
        provenance: 'direct',
      }],
      conditionalConsequences: [{
        canonicalUnit: 'cache.service', sources: ['ssh.service'],
        classification: 'retainedByUnaffectedReference',
      }],
      conditionalDiagnostics: [
        { canonicalUnit: 'ssh.service', kind: 'onFailureActivation', relatedUnit: 'recovery.service', jobMode: null, managerAction: null },
        { canonicalUnit: 'ssh.service', kind: 'upheldByReactivation', relatedUnit: 'guardian.service', jobMode: null, managerAction: null },
        { canonicalUnit: 'ssh.service', kind: 'successManagerAction', relatedUnit: null, jobMode: null, managerAction: 'reboot' },
      ],
      accounting: {
        observedUnitRecords: 1, observedRelationshipReferences: 0,
        observedCandidateReferences: 0, retainedUnits: 1, retainedEdges: 0,
        retainedCandidates: 0, retainedDiagnostics: 3, omittedKnownUnits: 0,
        omittedKnownEdges: 0, omittedKnownCandidates: 0, omittedKnownDiagnostics: 0,
        unresolvedFrontierReferences: 2, actualSshQueries: 2,
      },
      warnings: ['concurrentTopologyChange'],
      limitations: ['systemd topology only'],
    });
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Assess stop impact' }));
    expect(mocks.assessStopImpact).toHaveBeenCalledExactlyOnceWith(host.id, A, 'inspection-a');
    const dialog = await screen.findByRole('dialog', { name: 'Read-only stop impact' });
    expect(within(dialog).getByText(/does not authorize or perform a stop/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Absence of an observed relationship/)).toBeInTheDocument();
    expect(within(dialog).getAllByText('ssh.service')).toHaveLength(7);
    expect(within(dialog).getByText('requiredBy')).toBeInTheDocument();
    expect(within(dialog).getByText('worker.service')).toBeInTheDocument();
    expect(within(dialog).getByText('retainedByUnaffectedReference')).toBeInTheDocument();
    expect(within(dialog).getByText('cache.service')).toBeInTheDocument();
    expect(within(dialog).getByText('onFailureActivation')).toBeInTheDocument();
    expect(within(dialog).getByText('recovery.service')).toBeInTheDocument();
    expect(within(dialog).getByText('upheldByReactivation')).toBeInTheDocument();
    expect(within(dialog).getByText('guardian.service')).toBeInTheDocument();
    expect(within(dialog).getByText('successManagerAction')).toBeInTheDocument();
    expect(within(dialog).getByText('reboot')).toBeInTheDocument();
    expect(within(dialog).getByText('Retained passive diagnostics')).toBeInTheDocument();
    expect(within(dialog).getByText('Omitted known passive diagnostics')).toBeInTheDocument();
    expect(within(dialog).getByText(/do not prove causation/)).toBeInTheDocument();
    expect(within(dialog).queryByRole('button', { name: /stop/i })).not.toBeInTheDocument();
    expect(within(dialog).getByText(/concurrentTopologyChange/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog', { name: 'Read-only stop impact' })).not.toBeInTheDocument();
  });

  it('drops delayed stop-impact results after session turnover', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const value = snapshot();
    value.entries[0]!.stopImpactInspectionId = 'inspection-a';
    const delayed = deferred<unknown>();
    mocks.list.mockResolvedValueOnce(value).mockResolvedValueOnce(snapshot(host.id, B));
    mocks.assessStopImpact.mockReturnValueOnce(delayed.promise);
    const view = render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Assess stop impact' }));
    mocks.state = 'connected'; mocks.sessionId = B;
    view.rerender(<ServicesWorkspace host={host} />);
    await act(async () => { delayed.resolve({
      hostId: host.id, hostSessionId: A, rootUnit: 'ssh.service', observedAt: '',
      completeness: 'complete', uncertainty: 'directOnly', rootConsistent: true,
      units: [], edges: [], conditionalConsequences: [], conditionalDiagnostics: [], warnings: [], limitations: [], accounting: {
        observedUnitRecords: 1, observedRelationshipReferences: 0,
        observedCandidateReferences: 0, retainedUnits: 1, retainedEdges: 0,
        retainedCandidates: 0, retainedDiagnostics: 0, omittedKnownUnits: 0,
        omittedKnownEdges: 0, omittedKnownCandidates: 0, omittedKnownDiagnostics: 0,
        unresolvedFrontierReferences: 0, actualSshQueries: 2,
      },
    }); });
    expect(screen.queryByRole('dialog', { name: 'Read-only stop impact' })).not.toBeInTheDocument();
  });

  it('does not reopen diagnostics when assessment settles before an in-flight refresh', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const value = snapshot();
    value.entries[0]!.stopImpactInspectionId = 'inspection-a';
    const assessment = deferred<SystemdStopImpactAssessment>();
    const refresh = deferred<ServiceSnapshot>();
    mocks.list.mockResolvedValueOnce(value).mockReturnValueOnce(refresh.promise);
    mocks.assessStopImpact.mockReturnValueOnce(assessment.promise);
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Assess stop impact' }));
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await act(async () => { assessment.resolve(stopImpactAssessment()); });
    expect(screen.queryByRole('dialog', { name: 'Read-only stop impact' })).not.toBeInTheDocument();
    await act(async () => { refresh.resolve(value); });
    expect(screen.queryByRole('dialog', { name: 'Read-only stop impact' })).not.toBeInTheDocument();
  });

  it('dismisses a visible assessment as soon as refresh begins', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const value = snapshot();
    value.entries[0]!.stopImpactInspectionId = 'inspection-a';
    const refresh = deferred<ServiceSnapshot>();
    mocks.list.mockResolvedValueOnce(value).mockReturnValueOnce(refresh.promise);
    mocks.assessStopImpact.mockResolvedValueOnce(stopImpactAssessment());
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Assess stop impact' }));
    expect(await screen.findByRole('dialog', { name: 'Read-only stop impact' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(screen.queryByRole('dialog', { name: 'Read-only stop impact' })).not.toBeInTheDocument();
    await act(async () => { refresh.resolve(value); });
  });

  it('does not reopen diagnostics when refresh settles before the invalidated assessment', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const value = snapshot();
    value.entries[0]!.stopImpactInspectionId = 'inspection-a';
    const assessment = deferred<SystemdStopImpactAssessment>();
    const refresh = deferred<ServiceSnapshot>();
    mocks.list.mockResolvedValueOnce(value).mockReturnValueOnce(refresh.promise);
    mocks.assessStopImpact.mockReturnValueOnce(assessment.promise);
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Assess stop impact' }));
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    const assessButton = screen.getByRole('button', { name: 'Assess stop impact' });
    expect(assessButton).toBeDisabled();
    await userEvent.click(assessButton);
    expect(mocks.assessStopImpact).toHaveBeenCalledTimes(1);
    await act(async () => { refresh.resolve(value); });
    await act(async () => { assessment.resolve(stopImpactAssessment()); });
    expect(screen.queryByRole('dialog', { name: 'Read-only stop impact' })).not.toBeInTheDocument();
  });

  it('distinguishes empty, unavailable and failed refresh with same-session snapshot', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({ ...snapshot(), entries: [] })
      .mockRejectedValueOnce(new Error('unavailable'));
    const view = render(<ServicesWorkspace host={host} />);
    expect(await screen.findByText('No loaded system services were returned.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText('Refresh failed — showing the last successful snapshot.')).toBeInTheDocument();
    expect(screen.getByText('No loaded system services were returned.')).toBeInTheDocument();
    view.unmount();
    mocks.list.mockRejectedValueOnce(new Error('unavailable'));
    render(<ServicesWorkspace host={host} />);
    expect(await screen.findByText('Service inventory is unavailable on this host.')).toBeInTheDocument();
  });

  it('drops delayed old-session and old-host responses across reconnect, switch and unmount', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const old = deferred<ServiceSnapshot>();
    mocks.list.mockReturnValueOnce(old.promise).mockResolvedValueOnce(snapshot(host.id, B));
    const view = render(<ServicesWorkspace host={host} />);
    mocks.sessionId = B;
    view.rerender(<ServicesWorkspace host={host} />);
    await act(async () => { old.resolve(snapshot()); });
    expect(await screen.findByText('ssh.service')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(host.id, B);
    const switched = deferred<ServiceSnapshot>();
    mocks.list.mockReturnValueOnce(switched.promise).mockResolvedValueOnce(snapshot(otherHost.id, B));
    view.rerender(<ServicesWorkspace host={otherHost} />);
    expect(screen.queryByText('ssh.service')).not.toBeInTheDocument();
    view.unmount();
    await act(async () => { switched.resolve(snapshot(otherHost.id, B)); });
  });

  it('does not show an old snapshot after disconnect or accept mismatched backend identity', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce(snapshot(host.id, B));
    const view = render(<ServicesWorkspace host={host} />);
    expect(await screen.findByText('ssh.service')).toBeInTheDocument();
    mocks.state = 'disconnected'; mocks.sessionId = null;
    view.rerender(<ServicesWorkspace host={host} />);
    expect(screen.queryByText('ssh.service')).not.toBeInTheDocument();
    mocks.state = 'connected'; mocks.sessionId = A;
    view.rerender(<ServicesWorkspace host={host} />);
    expect(await screen.findByText('Service inventory is unavailable on this host.')).toBeInTheDocument();
    expect(within(screen.getByRole('button', { name: 'Refresh' })).queryByText('Refresh')).toBeInTheDocument();
  });

  it('shows actions only for backend observations and discards Cancel, close or Escape exactly once', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const observed = {
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', canStart: false, canReload: false, resetFailedObservationId: 'observation-a', tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }, {
        unit: 'text-only.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'No backend authority', canStart: false, canReload: false, resetFailedObservationId: null, tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    };
    mocks.list.mockResolvedValueOnce(observed);
    mocks.planResetFailed.mockResolvedValueOnce({
      planId: 'plan-a', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    }).mockResolvedValueOnce({
      planId: 'plan-b', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    }).mockResolvedValueOnce({
      planId: 'plan-c', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    });
    render(<ServicesWorkspace host={host} />);
    const firstAction = await screen.findByRole('button', { name: 'Reset failed state' });
    expect(screen.getAllByRole('button', { name: 'Reset failed state' })).toHaveLength(1);
    await userEvent.click(firstAction);
    const firstDialog = await screen.findByRole('dialog', { name: 'Approve systemd operation' });
    expect(within(firstDialog).getByText('Alpha')).toBeInTheDocument();
    expect(within(firstDialog).getByText('broken.service')).toBeInTheDocument();
    expect(within(firstDialog).getByText('loaded / failed / failed')).toBeInTheDocument();
    expect(within(firstDialog).getByText('Moderate')).toBeInTheDocument();
    expect(within(firstDialog).getByText('Clear only the failed-state marker.')).toBeInTheDocument();
    expect(mocks.planResetFailed).toHaveBeenCalledExactlyOnceWith(host.id, A, 'observation-a');
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(mocks.discardResetFailed).toHaveBeenCalledExactlyOnceWith(host.id, A, 'plan-a'));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const secondAction = screen.getByRole('button', { name: 'Reset failed state' });
    await waitFor(() => expect(secondAction).toBeEnabled());
    await userEvent.click(secondAction);
    await userEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Close dialog' }));
    await waitFor(() => expect(mocks.discardResetFailed).toHaveBeenLastCalledWith(host.id, A, 'plan-b'));
    expect(mocks.discardResetFailed).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(secondAction).toBeEnabled());
    await userEvent.click(secondAction);
    const thirdDialog = await screen.findByRole('dialog');
    fireEvent(thirdDialog, new Event('cancel', { cancelable: true }));
    await waitFor(() => expect(mocks.discardResetFailed).toHaveBeenLastCalledWith(host.id, A, 'plan-c'));
    expect(mocks.discardResetFailed).toHaveBeenCalledTimes(3);
  });

  it('requires explicit approval and executes a live plan exactly once', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', canStart: false, canReload: false, resetFailedObservationId: 'observation-a', tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    });
    mocks.planResetFailed.mockResolvedValueOnce({
      planId: 'plan-a', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    });
    const execution = deferred<{
      outcome: 'success'; auditStatus: 'persisted'; postObservationStatus: 'refreshed'; snapshot: ServiceSnapshot;
    }>();
    mocks.executeResetFailed.mockReturnValueOnce(execution.promise);
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset failed state' }));
    const approve = within(await screen.findByRole('dialog')).getByRole('button', { name: 'Reset failed state' });
    expect(approve).toBeDisabled();
    await userEvent.click(screen.getByRole('checkbox'));
    await userEvent.click(approve);
    await userEvent.click(approve);
    expect(mocks.executeResetFailed).toHaveBeenCalledExactlyOnceWith(host.id, A, 'plan-a');
    await act(async () => execution.resolve({
      outcome: 'success', auditStatus: 'persisted', postObservationStatus: 'refreshed', snapshot: snapshot(),
    }));
    expect(await screen.findByText('The failed-state marker was cleared.')).toBeInTheDocument();
    expect(mocks.discardResetFailed).not.toHaveBeenCalled();
  });

  it('requires High-risk confirmation and executes try-restart exactly once', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running',
        description: 'OpenSSH server', canStart: false, canReload: false, resetFailedObservationId: null, tryRestartObservationId: 'restart-observation', reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    });
    mocks.planTryRestart.mockResolvedValueOnce({
      planId: 'restart-plan', hostId: host.id, hostSessionId: A, unit: 'ssh.service',
      loadState: 'loaded', activeState: 'active', subState: 'running', risk: 'high',
      expiresInSeconds: 120n,
      effect: 'Restart may interrupt the service and does not guarantee application health.',
    });
    mocks.executeTryRestart.mockResolvedValueOnce({
      outcome: 'success', auditStatus: 'persisted', postObservationStatus: 'refreshed', snapshot: snapshot(),
    });
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Restart active service' }));
    const dialog = await screen.findByRole('dialog', { name: 'Approve systemd operation' });
    expect(within(dialog).getByText('High')).toBeInTheDocument();
    expect(within(dialog).getByText('loaded / active / running')).toBeInTheDocument();
    expect(within(dialog).getByText(/does not guarantee application health/)).toBeInTheDocument();
    const execute = within(dialog).getByRole('button', { name: 'Restart active service' });
    expect(execute).toBeDisabled();
    await userEvent.click(within(dialog).getByRole('checkbox'));
    await userEvent.click(execute);
    await userEvent.click(execute);
    expect(mocks.planTryRestart).toHaveBeenCalledExactlyOnceWith(host.id, A, 'restart-observation');
    expect(mocks.executeTryRestart).toHaveBeenCalledExactlyOnceWith(host.id, A, 'restart-plan');
    expect(await screen.findByText(/systemd try-restart operation completed/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });

  it('discards the exact try-restart plan on Cancel without executing it', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running',
        description: 'OpenSSH server', canStart: false, canReload: false, resetFailedObservationId: null, tryRestartObservationId: 'restart-observation', reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    });
    mocks.planTryRestart.mockResolvedValueOnce({
      planId: 'restart-plan', hostId: host.id, hostSessionId: A, unit: 'ssh.service',
      loadState: 'loaded', activeState: 'active', subState: 'running', risk: 'high',
      expiresInSeconds: 120n, effect: 'Restart may interrupt traffic.',
    });
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Restart active service' }));
    await userEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(mocks.discardTryRestart).toHaveBeenCalledExactlyOnceWith(host.id, A, 'restart-plan'));
    expect(mocks.executeTryRestart).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('keeps reload separate from try-restart and executes the exact approved reload once', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running',
        description: 'OpenSSH server', canStart: false, canReload: true, resetFailedObservationId: null,
        tryRestartObservationId: 'restart-observation', reloadObservationId: 'reload-observation', startObservationId: null, stopImpactInspectionId: null,
      }],
    });
    mocks.planReload.mockResolvedValueOnce({
      planId: 'reload-plan', hostId: host.id, hostSessionId: A, unit: 'ssh.service',
      loadState: 'loaded', activeState: 'active', subState: 'running', canReload: true,
      risk: 'high', expiresInSeconds: 120n, effect: 'Reload the running service configuration.',
    });
    mocks.executeReload.mockResolvedValueOnce({
      outcome: 'success', auditStatus: 'persisted', postObservationStatus: 'refreshed', snapshot: snapshot(),
    });
    render(<ServicesWorkspace host={host} />);
    expect(await screen.findByRole('button', { name: 'Restart active service' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reload service' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('High')).toBeInTheDocument();
    expect(within(dialog).getByText('Reload support')).toBeInTheDocument();
    expect(within(dialog).getByText('yes')).toBeInTheDocument();
    expect(within(dialog).getByText(/Reload the running service configuration/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('checkbox'));
    const execute = within(dialog).getByRole('button', { name: 'Reload service' });
    await userEvent.click(execute);
    await userEvent.click(execute);
    expect(mocks.planReload).toHaveBeenCalledExactlyOnceWith(host.id, A, 'reload-observation');
    expect(mocks.executeReload).toHaveBeenCalledExactlyOnceWith(host.id, A, 'reload-plan');
    expect(mocks.executeTryRestart).not.toHaveBeenCalled();
    expect(await screen.findByText(/systemd reload operation completed/)).toBeInTheDocument();
  });

  it('discards the exact reload plan on Cancel without executing it', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running',
        description: 'OpenSSH server', canStart: false, canReload: true, resetFailedObservationId: null,
        tryRestartObservationId: 'restart-observation', reloadObservationId: 'reload-observation', startObservationId: null, stopImpactInspectionId: null,
      }],
    });
    mocks.planReload.mockResolvedValueOnce({
      planId: 'reload-plan', hostId: host.id, hostSessionId: A, unit: 'ssh.service',
      loadState: 'loaded', activeState: 'active', subState: 'running', canReload: true,
      risk: 'high', expiresInSeconds: 120n, effect: 'Reload the running service configuration.',
    });
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reload service' }));
    await userEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(mocks.discardReload).toHaveBeenCalledExactlyOnceWith(host.id, A, 'reload-plan'));
    expect(mocks.executeReload).not.toHaveBeenCalled();
    expect(mocks.discardTryRestart).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('discards a delayed plan with its old session identity and never renders it after reconnect', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', canStart: false, canReload: false, resetFailedObservationId: 'observation-a', tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    }).mockResolvedValueOnce(snapshot(host.id, B));
    const delayed = deferred<{
      planId: string; hostId: string; hostSessionId: string; unit: string; loadState: string;
      activeState: string; subState: string; risk: 'moderate'; expiresInSeconds: bigint; effect: string;
    }>();
    mocks.planResetFailed.mockReturnValueOnce(delayed.promise);
    const view = render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset failed state' }));
    mocks.sessionId = B;
    view.rerender(<ServicesWorkspace host={host} />);
    await act(async () => delayed.resolve({
      planId: 'old-plan', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    }));
    await waitFor(() => expect(mocks.discardResetFailed).toHaveBeenCalledExactlyOnceWith(host.id, A, 'old-plan'));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mocks.executeResetFailed).not.toHaveBeenCalled();
  });

  it('discards a visible plan on session turnover and never restores its modal when the old owner returns', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const observed = {
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', canStart: false, canReload: false, resetFailedObservationId: 'observation-a', tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    };
    mocks.list.mockResolvedValueOnce(observed)
      .mockResolvedValueOnce(snapshot(host.id, B))
      .mockResolvedValueOnce(observed);
    mocks.planResetFailed.mockResolvedValueOnce({
      planId: 'plan-a', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    });
    const view = render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset failed state' }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();

    mocks.sessionId = B;
    view.rerender(<ServicesWorkspace host={host} />);
    await waitFor(() => expect(mocks.discardResetFailed).toHaveBeenCalledExactlyOnceWith(host.id, A, 'plan-a'));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    mocks.sessionId = A;
    view.rerender(<ServicesWorkspace host={host} />);
    await screen.findByRole('button', { name: 'Reset failed state' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mocks.executeResetFailed).not.toHaveBeenCalled();
  });

  it('keeps unknown mutation, audit failure and post-observation failure as separate terminal truths', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', canStart: false, canReload: false, resetFailedObservationId: 'observation-a', tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    });
    mocks.planResetFailed.mockResolvedValueOnce({
      planId: 'plan-a', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    });
    mocks.executeResetFailed.mockResolvedValueOnce({
      outcome: 'outcomeUnknown', auditStatus: 'failed', postObservationStatus: 'unavailable', snapshot: null,
    });
    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset failed state' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.click(within(dialog).getByRole('checkbox'));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Reset failed state' }));
    expect(await screen.findByText(/The connection ended before the remote outcome could be confirmed/)).toHaveTextContent(
      'The local audit record could not be persisted. The post-operation service refresh was unavailable.',
    );
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });

  it('keeps Start operation-specific, High-risk, one-shot and session-owned', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const observed = {
      ...snapshot(),
      entries: [
        {
          unit: 'inactive-startable.service', loadState: 'loaded', activeState: 'inactive', subState: 'dead',
          description: 'Startable fixture', canStart: true, canReload: false,
          resetFailedObservationId: null, tryRestartObservationId: null, reloadObservationId: null,
          startObservationId: 'start-observation', stopImpactInspectionId: null,
        },
        {
          unit: 'inactive-disabled.service', loadState: 'loaded', activeState: 'inactive', subState: 'dead',
          description: 'Not startable', canStart: false, canReload: false,
          resetFailedObservationId: null, tryRestartObservationId: null, reloadObservationId: null,
          startObservationId: null, stopImpactInspectionId: null,
        },
        {
          unit: 'running.service', loadState: 'loaded', activeState: 'active', subState: 'running',
          description: 'Running', canStart: true, canReload: false,
          resetFailedObservationId: null, tryRestartObservationId: 'restart-observation', reloadObservationId: null,
          startObservationId: null, stopImpactInspectionId: null,
        },
        {
          unit: 'failed.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
          description: 'Failed', canStart: true, canReload: false,
          resetFailedObservationId: 'reset-observation', tryRestartObservationId: null, reloadObservationId: null,
          startObservationId: null, stopImpactInspectionId: null,
        },
      ],
    } satisfies ServiceSnapshot;
    const plan = {
      planId: 'start-plan', hostId: host.id, hostSessionId: A,
      unit: 'inactive-startable.service', loadState: 'loaded', activeState: 'inactive', subState: 'dead',
      canStart: true, risk: 'high', expiresInSeconds: 120n,
      effect: 'Requests systemd to start the selected inactive service.',
    } as const;
    mocks.list.mockResolvedValue(observed);
    mocks.planStart.mockResolvedValue(plan);
    mocks.executeStart.mockResolvedValue({
      outcome: 'success', auditStatus: 'persisted', postObservationStatus: 'refreshed', snapshot: observed,
    });
    render(<ServicesWorkspace host={host} />);
    const startButton = await screen.findByRole('button', { name: 'Start service' });
    expect(screen.getAllByRole('button', { name: 'Start service' })).toHaveLength(1);
    expect(screen.getByRole('button', { name: 'Restart active service' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reset failed state' })).toBeInTheDocument();
    await userEvent.click(startButton);
    expect(mocks.planStart).toHaveBeenCalledExactlyOnceWith(host.id, A, 'start-observation');
    expect(mocks.planResetFailed).not.toHaveBeenCalled();
    expect(mocks.planTryRestart).not.toHaveBeenCalled();
    expect(mocks.planReload).not.toHaveBeenCalled();
    let dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('inactive-startable.service')).toBeInTheDocument();
    expect(within(dialog).getByText('loaded / inactive / dead')).toBeInTheDocument();
    expect(within(dialog).getByText('Start support')).toBeInTheDocument();
    expect(within(dialog).getByText('High')).toBeInTheDocument();
    expect(within(dialog).getAllByText(/may activate dependencies/)).toHaveLength(2);
    expect(within(dialog).getByRole('button', { name: 'Start service' })).toBeDisabled();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(mocks.discardStart).toHaveBeenCalledExactlyOnceWith(host.id, A, 'start-plan'));
    expect(mocks.executeStart).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: 'Start service' }));
    dialog = await screen.findByRole('dialog');
    const confirm = within(dialog).getByRole('button', { name: 'Start service' });
    await userEvent.click(within(dialog).getByRole('checkbox'));
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);
    await waitFor(() => expect(mocks.executeStart).toHaveBeenCalledExactlyOnceWith(host.id, A, 'start-plan'));
    expect(await screen.findByText('The systemd start operation completed. The refreshed service state is shown below.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });

  it('renders safe failed and cancelled terminal outcomes without reusable authority', async () => {
    const observed = {
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', canStart: false, canReload: false, resetFailedObservationId: 'observation-a', tryRestartObservationId: null, reloadObservationId: null, startObservationId: null, stopImpactInspectionId: null,
      }],
    };
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValue(observed);
    mocks.planResetFailed.mockResolvedValueOnce({
      planId: 'plan-failed', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    }).mockResolvedValueOnce({
      planId: 'plan-cancelled', hostId: host.id, hostSessionId: A, unit: 'broken.service',
      loadState: 'loaded', activeState: 'failed', subState: 'failed', risk: 'moderate',
      expiresInSeconds: 120n, effect: 'Clear only the failed-state marker.',
    });
    mocks.executeResetFailed.mockResolvedValueOnce({
      outcome: 'failed', auditStatus: 'persisted', postObservationStatus: 'unavailable', snapshot: null,
    }).mockResolvedValueOnce({
      outcome: 'cancelled', auditStatus: 'persisted', postObservationStatus: 'unavailable', snapshot: null,
    });

    const failedView = render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset failed state' }));
    let dialog = await screen.findByRole('dialog');
    await userEvent.click(within(dialog).getByRole('checkbox'));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Reset failed state' }));
    expect(await screen.findByText(/The service operation failed/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
    failedView.unmount();

    render(<ServicesWorkspace host={host} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset failed state' }));
    dialog = await screen.findByRole('dialog');
    await userEvent.click(within(dialog).getByRole('checkbox'));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Reset failed state' }));
    expect(await screen.findByText(/The service operation was cancelled before dispatch/)).toBeInTheDocument();
    expect(mocks.executeResetFailed).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('button', { name: /retry/i })).not.toBeInTheDocument();
  });
});
