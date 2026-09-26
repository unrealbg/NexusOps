import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Host, ServiceSnapshot } from '@nexusops/protocol';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  state: 'disconnected',
  sessionId: null as string | null,
  list: vi.fn(),
}));
vi.mock('../../api/queries', () => ({
  useHostSession: () => ({ data: { state: mocks.state, hostSessionId: mocks.sessionId } }),
}));
vi.mock('../../api/client', () => ({ servicesApi: { list: mocks.list } }));

import { ServicesWorkspace } from './ServicesWorkspace';

const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const host = { id: 'host-a', displayName: 'Alpha' } as Host;
const otherHost = { id: 'host-b', displayName: 'Beta' } as Host;
function snapshot(hostId = host.id, hostSessionId = A): ServiceSnapshot {
  return {
    hostId, hostSessionId, observedAt: '2026-09-26T12:00:00Z',
    entries: [
      { unit: 'ssh.service', loadState: 'loaded', activeState: 'active', subState: 'running', description: 'OpenSSH server' },
      { unit: 'future.service', loadState: 'loaded', activeState: 'repairing', subState: 'unknown', description: '<script>bad</script> text' },
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
});
