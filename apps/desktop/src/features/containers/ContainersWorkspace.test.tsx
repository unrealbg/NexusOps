import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';
import userEvent from '@testing-library/user-event';
import type { ContainerSnapshot, ContainerState, Host, HostSession } from '@nexusops/protocol';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  sessionQuery: { data: undefined as HostSession | undefined, isError: false, isPending: false },
  list: vi.fn(),
}));
vi.mock('../../api/queries', () => ({ useHostSession: () => mocks.sessionQuery }));
vi.mock('../../api/client', () => ({ containersApi: { list: mocks.list } }));
import { ContainersWorkspace } from './ContainersWorkspace';

const A = 'session-a';
const B = 'session-b';
const host = { id: 'host-a', displayName: 'Alpha' } as Host;
const otherHost = { id: 'host-b', displayName: 'Beta' } as Host;
function session(hostId = host.id, hostSessionId: string | null = A, state: HostSession['state'] = 'connected'): HostSession {
  return { hostId, hostSessionId, state, error: null, identity: null, capabilities: [], discovery: null };
}
function snapshot(hostId = host.id, hostSessionId = A, name = 'synthetic-container'): ContainerSnapshot {
  return { hostId, hostSessionId, observedAt: '2026-09-28T10:00:00Z', provider: 'dockerSystem', entries: [
    { id: 'a'.repeat(64), name, image: 'example/image:1', state: 'running', status: 'Up 2 minutes', ports: '', networks: 'bridge' },
  ] };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  mocks.sessionQuery = { data: session(), isError: false, isPending: false };
  mocks.list.mockReset();
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

describe('session-owned Docker system inventory', () => {
  it('issues one initial request under StrictMode effect replay', async () => {
    mocks.list.mockResolvedValue(snapshot());
    const view = render(<StrictMode><ContainersWorkspace host={host} /></StrictMode>);
    expect(await screen.findByText('synthetic-container')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    view.rerender(<StrictMode><ContainersWorkspace host={{ ...host, displayName: 'Renamed' }} /></StrictMode>);
    expect(mocks.list).toHaveBeenCalledTimes(1);
  });

  it('makes no Docker request while disconnected or missing session identity', () => {
    mocks.sessionQuery.data = session(host.id, null, 'disconnected');
    const view = render(<ContainersWorkspace host={host} />);
    expect(screen.getByText('Connect this host to inspect local Docker containers.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    mocks.sessionQuery.data = session(host.id, null);
    view.rerender(<ContainersWorkspace host={host} />);
    expect(screen.getByText('Current SSH session state is unavailable.')).toBeInTheDocument();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('makes no request or Connected claim during a pending session query', async () => {
    mocks.sessionQuery = { data: undefined, isError: false, isPending: true };
    await act(async () => { render(<ContainersWorkspace host={host} />); });
    expect(screen.getByText('Reading connection state…')).toBeInTheDocument();
    expect(screen.queryByText(/· Connected/)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it.each(['error with retained data', 'wrong host', 'missing data'])('fails closed on initial session query %s', async (reason) => {
    if (reason === 'error with retained data') mocks.sessionQuery.isError = true;
    if (reason === 'wrong host') mocks.sessionQuery.data = session(otherHost.id);
    if (reason === 'missing data') mocks.sessionQuery.data = undefined;
    await act(async () => { render(<ContainersWorkspace host={host} />); });
    expect(screen.getByText('Current SSH session state is unavailable.')).toBeInTheDocument();
    expect(screen.queryByText(/· Connected/)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('discards rows and filter immediately when the session query errors, then requires a fresh read', async () => {
    const oldRefresh = deferred<ContainerSnapshot>();
    const fresh = deferred<ContainerSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(oldRefresh.promise).mockReturnValueOnce(fresh.promise);
    const view = render(<ContainersWorkspace host={host} />);
    await screen.findByText('synthetic-container');
    await userEvent.type(screen.getByRole('searchbox'), 'synthetic');
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    mocks.sessionQuery.isError = true;
    view.rerender(<ContainersWorkspace host={host} />);
    expect(screen.queryByText('synthetic-container')).not.toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    mocks.sessionQuery.isError = false;
    view.rerender(<ContainersWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(3));
    expect(screen.queryByText('synthetic-container')).not.toBeInTheDocument();
    await act(async () => { oldRefresh.resolve(snapshot(host.id, A, 'revoked-container')); });
    expect(screen.queryByText('revoked-container')).not.toBeInTheDocument();
    await act(async () => { fresh.resolve(snapshot(host.id, A, 'fresh-container')); });
    expect(screen.getByText('fresh-container')).toBeInTheDocument();
    expect(screen.getByRole('searchbox')).toHaveValue('');
  });

  it('filters only in memory and renders untrusted metadata as inert text without actions', async () => {
    const hostile = '<img src=x onerror=alert(1)> https://example.test';
    mocks.list.mockResolvedValue(snapshot(host.id, A, hostile));
    const storage = vi.spyOn(Storage.prototype, 'setItem');
    const { container } = render(<ContainersWorkspace host={host} />);
    expect(await screen.findByText(hostile)).toBeInTheDocument();
    expect(screen.getByText('aaaaaaaaaaaa')).toBeInTheDocument();
    expect(screen.getByText('Docker · local system daemon · Up to 64 most recently created containers in all states. Refresh manually to update.')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Filter this snapshot' }), 'missing');
    expect(screen.getByText('No containers match this filter.')).toBeInTheDocument();
    await userEvent.clear(screen.getByRole('searchbox'));
    expect(screen.getByText(hostile)).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(container.querySelector('img, a')).toBeNull();
    expect(screen.queryByRole('button', { name: /copy|export|start|stop|inspect|exec|stats/i })).not.toBeInTheDocument();
    expect(storage).not.toHaveBeenCalled();
  });

  it('does not poll or refetch on window focus during 65 seconds', async () => {
    vi.useFakeTimers();
    mocks.list.mockResolvedValue(snapshot());
    await act(async () => { render(<ContainersWorkspace host={host} />); });
    expect(screen.getByText('synthetic-container')).toBeInTheDocument();
    await act(async () => { await vi.advanceTimersByTimeAsync(65_000); window.dispatchEvent(new Event('focus')); });
    expect(mocks.list).toHaveBeenCalledTimes(1);
  });

  it('refreshes once despite repeated input and retains a timestamped same-session snapshot on failure', async () => {
    const failure = deferred<ContainerSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(failure.promise);
    render(<ContainersWorkspace host={host} />);
    await screen.findByText('synthetic-container');
    const button = screen.getByRole('button', { name: 'Refresh' });
    fireEvent.click(button); fireEvent.click(button);
    expect(button).toBeDisabled();
    expect(mocks.list).toHaveBeenCalledTimes(2);
    await act(async () => { failure.reject(new Error('private Docker diagnostic')); });
    expect(screen.getByText('Refresh failed — showing the last successful snapshot.')).toBeInTheDocument();
    expect(screen.getByText('synthetic-container')).toBeInTheDocument();
    expect(screen.getByText('2026-09-28T10:00:00Z')).toBeInTheDocument();
    expect(screen.queryByText('private Docker diagnostic')).not.toBeInTheDocument();
    expect(button).toBeEnabled();
  });

  it('uses factual empty wording and fixed initial error with manual recovery', async () => {
    mocks.list.mockResolvedValueOnce({ ...snapshot(), entries: [] });
    const view = render(<ContainersWorkspace host={host} />);
    expect(await screen.findByText('No containers were returned by the local Docker daemon.')).toBeInTheDocument();
    expect(screen.queryByText('Docker is not installed.')).not.toBeInTheDocument();
    view.unmount();
    mocks.list.mockReset().mockRejectedValueOnce(new Error('socket details')).mockResolvedValueOnce(snapshot());
    render(<ContainersWorkspace host={host} />);
    expect(await screen.findByText('Docker container inventory is unavailable for this connection.')).toBeInTheDocument();
    expect(screen.queryByText('socket details')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText('synthetic-container')).toBeInTheDocument();
  });

  it('removes rows on disconnect and requires a new read after reconnect', async () => {
    const next = deferred<ContainerSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(next.promise);
    const view = render(<ContainersWorkspace host={host} />);
    await screen.findByText('synthetic-container');
    mocks.sessionQuery.data = session(host.id, null, 'disconnected');
    view.rerender(<ContainersWorkspace host={host} />);
    expect(screen.queryByText('synthetic-container')).not.toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    mocks.sessionQuery.data = session(host.id, B);
    view.rerender(<ContainersWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(2));
    expect(mocks.list).toHaveBeenLastCalledWith(host.id, B);
    expect(screen.queryByText('synthetic-container')).not.toBeInTheDocument();
    await act(async () => { next.resolve(snapshot(host.id, B, 'new-session-container')); });
    expect(screen.getByText('new-session-container')).toBeInTheDocument();
  });

  it('ignores delayed old-session and old-host results', async () => {
    const old = deferred<ContainerSnapshot>();
    const next = deferred<ContainerSnapshot>();
    mocks.list.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    const view = render(<ContainersWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(1));
    mocks.sessionQuery.data = session(otherHost.id);
    view.rerender(<ContainersWorkspace host={otherHost} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(2));
    await act(async () => { old.resolve(snapshot()); });
    expect(screen.queryByText('synthetic-container')).not.toBeInTheDocument();
    await act(async () => { next.resolve(snapshot(otherHost.id, A, 'beta-container')); });
    expect(screen.getByText('beta-container')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(otherHost.id, A);
  });

  it('ignores a delayed old-session result after reconnecting the same host', async () => {
    const old = deferred<ContainerSnapshot>();
    const fresh = deferred<ContainerSnapshot>();
    mocks.list.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const view = render(<ContainersWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A));
    mocks.sessionQuery.data = session(host.id, null, 'disconnected');
    view.rerender(<ContainersWorkspace host={host} />);
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    mocks.sessionQuery.data = session(host.id, B);
    view.rerender(<ContainersWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenLastCalledWith(host.id, B));
    await act(async () => { old.resolve(snapshot(host.id, A, 'old-session-container')); });
    expect(screen.queryByText('old-session-container')).not.toBeInTheDocument();
    await act(async () => { fresh.resolve(snapshot(host.id, B, 'current-session-container')); });
    expect(screen.getByText('current-session-container')).toBeInTheDocument();
  });

  it.each([[otherHost.id, A, 'dockerSystem'], [host.id, B, 'dockerSystem'], [host.id, A, 'invalid']])(
    'rejects mismatched result identity/provider %s/%s/%s', async (hostId, sessionId, provider) => {
      mocks.list.mockResolvedValue({ ...snapshot(hostId, sessionId), provider });
      render(<ContainersWorkspace host={host} />);
      expect(await screen.findByText('Docker container inventory is unavailable for this connection.')).toBeInTheDocument();
      expect(screen.queryByText('synthetic-container')).not.toBeInTheDocument();
    },
  );

  it('shows every typed Docker state as a factual label', async () => {
    const result = snapshot();
    const entry = result.entries[0];
    if (!entry) throw new Error('Missing synthetic entry');
    const states: ContainerState[] = ['created', 'restarting', 'running', 'removing', 'paused', 'exited', 'dead'];
    result.entries = states.map((state, index) => ({ ...entry, id: index.toString(16).repeat(64), name: `container-${index}`, state }));
    mocks.list.mockResolvedValue(result);
    render(<ContainersWorkspace host={host} />);
    await screen.findByText('container-0');
    for (const state of ['Created', 'Restarting', 'Running', 'Removing', 'Paused', 'Exited', 'Dead'])
      expect(screen.getByText(state)).toBeInTheDocument();
  });
});
