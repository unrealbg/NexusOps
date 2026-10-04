import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Host, ServiceSnapshot } from '@nexusops/protocol';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  state: 'disconnected',
  sessionId: null as string | null,
  list: vi.fn(),
  planResetFailed: vi.fn(),
  discardResetFailed: vi.fn(),
  executeResetFailed: vi.fn(),
}));
vi.mock('../../api/queries', () => ({
  useHostSession: () => ({ data: { state: mocks.state, hostSessionId: mocks.sessionId } }),
}));
vi.mock('../../api/client', () => ({ servicesApi: {
  list: mocks.list,
  planResetFailed: mocks.planResetFailed,
  discardResetFailed: mocks.discardResetFailed,
  executeResetFailed: mocks.executeResetFailed,
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
      { unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running', description: 'OpenSSH server', resetFailedObservationId: null },
      { unit: 'future.service', loadState: 'loaded', activeState: 'repairing', subState: 'unknown', description: '<script>bad</script> text', resetFailedObservationId: null },
    ],
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
  mocks.planResetFailed.mockReset();
  mocks.discardResetFailed.mockReset().mockResolvedValue(true);
  mocks.executeResetFailed.mockReset();
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
        description: 'Broken fixture', resetFailedObservationId: 'observation-a',
      }, {
        unit: 'text-only.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'No backend authority', resetFailedObservationId: null,
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
        description: 'Broken fixture', resetFailedObservationId: 'observation-a',
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

  it('discards a delayed plan with its old session identity and never renders it after reconnect', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', resetFailedObservationId: 'observation-a',
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
        description: 'Broken fixture', resetFailedObservationId: 'observation-a',
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
        description: 'Broken fixture', resetFailedObservationId: 'observation-a',
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

  it('renders safe failed and cancelled terminal outcomes without reusable authority', async () => {
    const observed = {
      ...snapshot(),
      entries: [{
        unit: 'broken.service', loadState: 'loaded', activeState: 'failed', subState: 'failed',
        description: 'Broken fixture', resetFailedObservationId: 'observation-a',
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
