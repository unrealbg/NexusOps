import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Host, NetworkSnapshot } from '@nexusops/protocol';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  state: 'disconnected',
  sessionId: null as string | null,
  list: vi.fn(),
}));
vi.mock('../../api/queries', () => ({
  useHostSession: () => ({ data: { state: mocks.state, hostSessionId: mocks.sessionId } }),
}));
vi.mock('../../api/client', () => ({ networkApi: { list: mocks.list } }));

import { NetworkWorkspace } from './NetworkWorkspace';

const A = '11111111-1111-4111-8111-111111111111';
const B = '22222222-2222-4222-8222-222222222222';
const host = { id: 'host-a', displayName: 'Alpha' } as Host;
const otherHost = { id: 'host-b', displayName: 'Beta' } as Host;
function snapshot(hostId = host.id, hostSessionId = A): NetworkSnapshot {
  return {
    hostId, hostSessionId, observedAt: '2026-09-26T12:00:00Z',
    entries: [
      { ifindex: 1, name: 'lo', operState: null, mtu: null,
        addresses: [{ family: 'ipv4', address: '127.0.0.1', prefixLength: 8 }] },
      { ifindex: 2, name: '<img src=x>', operState: 'FUTURE_STATE', mtu: 1500,
        addresses: [{ family: 'ipv6', address: '2001:db8::10', prefixLength: 64 }] },
    ],
  };
}
function freshSnapshot(hostSessionId: string): NetworkSnapshot {
  const result = snapshot(host.id, hostSessionId);
  result.entries[0] = { ifindex: 1, name: 'lo', operState: null, mtu: null,
    addresses: [{ family: 'ipv4', address: '127.0.0.2', prefixLength: 8 }] };
  return result;
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

describe('session-bound Network workspace', () => {
  it('makes no request for a disconnected host', () => {
    render(<NetworkWorkspace host={host} />);
    expect(screen.getByText('Connect this host to inspect network interfaces.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('loads once, renders validated text and optional fields, and filters without remote calls', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValue(snapshot());
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('<img src=x>')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    expect(screen.getByText('IPv6 2001:db8::10/64')).toBeInTheDocument();
    expect(screen.getByText('2 interfaces · 2 addresses', { exact: false })).toBeInTheDocument();
    expect(screen.getAllByText('—')).toHaveLength(2);
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    await userEvent.type(screen.getByRole('searchbox', { name: 'Filter interfaces and addresses' }), '2001:db8');
    expect(screen.queryByText('lo')).not.toBeInTheDocument();
    expect(screen.getByText('<img src=x>')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(1);
    await userEvent.clear(screen.getByRole('searchbox'));
    await userEvent.type(screen.getByRole('searchbox'), 'no-match');
    expect(screen.getByText('No interfaces match this filter.')).toBeInTheDocument();
  });

  it('refreshes once while pending and keeps only a same-session snapshot on failure', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const refresh = deferred<NetworkSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(refresh.promise);
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    const button = screen.getByRole('button', { name: 'Refresh' });
    await userEvent.click(button);
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    await act(async () => { refresh.reject(new Error('unavailable')); });
    expect(await screen.findByText('Refresh failed — showing the last successful snapshot.')).toBeInTheDocument();
    expect(screen.getByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    await waitFor(() => expect(button).toBeEnabled());
  });

  it('distinguishes an empty success from initial unavailability', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce({ ...snapshot(), entries: [] });
    const view = render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('No network interfaces were returned.')).toBeInTheDocument();
    view.unmount();
    mocks.list.mockRejectedValueOnce(new Error('unsupported'));
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('Network inventory is unavailable on this host.')).toBeInTheDocument();
  });

  it('hides disconnected data, requests a new session, and drops delayed old-session data', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const old = deferred<NetworkSnapshot>();
    const fresh = freshSnapshot(B);
    mocks.list.mockReturnValueOnce(old.promise).mockResolvedValueOnce(fresh);
    const view = render(<NetworkWorkspace host={host} />);
    mocks.state = 'disconnected'; mocks.sessionId = null;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.getByText('Connect this host to inspect network interfaces.')).toBeInTheDocument();
    mocks.state = 'connected'; mocks.sessionId = B;
    view.rerender(<NetworkWorkspace host={host} />);
    await act(async () => { old.resolve(snapshot()); });
    expect(await screen.findByText('IPv4 127.0.0.2/8')).toBeInTheDocument();
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(host.id, B);
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it('removes a visible snapshot on disconnect and does not reuse it on reconnect', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const next = deferred<NetworkSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(next.promise);
    const view = render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('IPv4 127.0.0.1/8')).toBeInTheDocument();
    mocks.state = 'disconnected'; mocks.sessionId = null;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    mocks.state = 'connected'; mocks.sessionId = B;
    view.rerender(<NetworkWorkspace host={host} />);
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    await act(async () => { next.resolve(freshSnapshot(B)); });
    expect(screen.getByText('IPv4 127.0.0.2/8')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it('drops old-host and unmounted responses', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    const oldHost = deferred<NetworkSnapshot>();
    const unmounted = deferred<NetworkSnapshot>();
    mocks.list.mockReturnValueOnce(oldHost.promise).mockReturnValueOnce(unmounted.promise);
    const view = render(<NetworkWorkspace host={host} />);
    view.rerender(<NetworkWorkspace host={otherHost} />);
    await act(async () => { oldHost.resolve(snapshot()); });
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(otherHost.id, A);
    view.unmount();
    await act(async () => { unmounted.resolve(snapshot(otherHost.id)); });
  });

  it('rejects mismatched host and session identities from the backend', async () => {
    mocks.state = 'connected'; mocks.sessionId = A;
    mocks.list.mockResolvedValueOnce(snapshot(otherHost.id, A));
    const view = render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('Network inventory is unavailable on this host.')).toBeInTheDocument();
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
    view.unmount();
    mocks.list.mockResolvedValueOnce(snapshot(host.id, B));
    render(<NetworkWorkspace host={host} />);
    expect(await screen.findByText('Network inventory is unavailable on this host.')).toBeInTheDocument();
    expect(screen.queryByText('IPv4 127.0.0.1/8')).not.toBeInTheDocument();
  });
});
