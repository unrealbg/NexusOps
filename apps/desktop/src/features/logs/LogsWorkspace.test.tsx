import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';
import userEvent from '@testing-library/user-event';
import type { Host, HostSession, JournalPriority, SystemJournalSnapshot } from '@nexusops/protocol';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  sessionQuery: { data: undefined as HostSession | undefined, isError: false, isPending: false },
  list: vi.fn(),
}));
vi.mock('../../api/queries', () => ({
  useHostSession: () => mocks.sessionQuery,
}));
vi.mock('../../api/client', () => ({ logsApi: { list: mocks.list } }));
import { LogsWorkspace } from './LogsWorkspace';

const A = 'session-a';
const B = 'session-b';
const host = { id: 'host-a', displayName: 'Alpha' } as Host;
const otherHost = { id: 'host-b', displayName: 'Beta' } as Host;
function session(hostId = host.id, hostSessionId: string | null = A, state: HostSession['state'] = 'connected'): HostSession {
  return { hostId, hostSessionId, state, error: null, identity: null, capabilities: [], discovery: null };
}
function snapshot(hostId = host.id, hostSessionId = A, message = 'synthetic journal row'): SystemJournalSnapshot {
  return { hostId, hostSessionId, observedAt: '2026-09-27T10:00:00Z', entries: [
    { timestamp: '2026-09-27T09:59:00.000001Z', priority: 'info', unit: 'example.service', identifier: 'example', messageState: 'text', message },
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

describe('session-owned Logs snapshot', () => {
  it('issues one initial request under the application StrictMode effect replay', async () => {
    mocks.list.mockResolvedValue(snapshot());
    const view = render(<StrictMode><LogsWorkspace host={host} /></StrictMode>);
    expect(await screen.findByText('synthetic journal row')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    view.rerender(<StrictMode><LogsWorkspace host={{ ...host, displayName: 'Renamed' }} /></StrictMode>);
    expect(mocks.list).toHaveBeenCalledTimes(1);
  });
  it('does not request journal data when disconnected or missing a session ID', () => {
    mocks.sessionQuery.data = session(host.id, null, 'disconnected');
    const view = render(<LogsWorkspace host={host} />);
    expect(screen.getByText('Connect this host to inspect recent system journal entries.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    mocks.sessionQuery.data = session(host.id, null);
    view.rerender(<LogsWorkspace host={host} />);
    expect(screen.getByText('Current SSH session state is unavailable.')).toBeInTheDocument();
    expect(screen.queryByText(/· (Connected|Disconnected)/)).not.toBeInTheDocument();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it.each(['error with retained data', 'wrong host', 'missing data'])('fails closed on initial session query %s', async (reason) => {
    mocks.list.mockResolvedValue(snapshot());
    if (reason === 'error with retained data') mocks.sessionQuery.isError = true;
    if (reason === 'wrong host') mocks.sessionQuery.data = session(otherHost.id);
    if (reason === 'missing data') mocks.sessionQuery.data = undefined;
    await act(async () => { render(<LogsWorkspace host={host} />); });
    expect(screen.getByText('Current SSH session state is unavailable.')).toBeInTheDocument();
    expect(screen.queryByText(/· (Connected|Disconnected)/)).not.toBeInTheDocument();
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('reports a pending session query without claiming a connection state or requesting Logs', async () => {
    mocks.sessionQuery = { data: undefined, isError: false, isPending: true };
    await act(async () => { render(<LogsWorkspace host={host} />); });
    expect(screen.getByText('Reading connection state…')).toBeInTheDocument();
    expect(screen.queryByText(/· (Connected|Disconnected)/)).not.toBeInTheDocument();
    expect(screen.queryByText('Current SSH session state is unavailable.')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it('removes the sensitive snapshot and filter immediately on a session query error with retained connected data', async () => {
    mocks.list.mockResolvedValue(snapshot());
    const view = render(<LogsWorkspace host={host} />);
    await screen.findByText('synthetic journal row');
    await userEvent.type(screen.getByRole('searchbox'), 'synthetic');
    expect(screen.getByText('synthetic journal row')).toBeInTheDocument();
    mocks.sessionQuery.isError = true;
    view.rerender(<LogsWorkspace host={host} />);
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    expect(screen.queryByText('2026-09-27T10:00:00Z')).not.toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    expect(screen.queryByText(/last successful snapshot/)).not.toBeInTheDocument();
    expect(screen.queryByText(/· (Connected|Disconnected)/)).not.toBeInTheDocument();
    expect(screen.getByText('Current SSH session state is unavailable.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
  });

  it('requires a fresh read after recovery to the same session and does not resurrect rows or filter state', async () => {
    const oldRefresh = deferred<SystemJournalSnapshot>();
    const freshRead = deferred<SystemJournalSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(oldRefresh.promise).mockReturnValueOnce(freshRead.promise);
    const view = render(<LogsWorkspace host={host} />);
    await screen.findByText('synthetic journal row');
    await userEvent.type(screen.getByRole('searchbox'), 'synthetic');
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    mocks.sessionQuery.isError = true;
    view.rerender(<LogsWorkspace host={host} />);
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(2);
    mocks.sessionQuery.isError = false;
    view.rerender(<LogsWorkspace host={host} />);
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(3));
    expect(mocks.list).toHaveBeenLastCalledWith(host.id, A);
    await act(async () => { oldRefresh.resolve(snapshot(host.id, A, 'revoked snapshot')); });
    expect(screen.queryByText('revoked snapshot')).not.toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    await act(async () => { freshRead.resolve(snapshot(host.id, A, 'confirmed fresh row')); });
    expect(screen.getByText('confirmed fresh row')).toBeInTheDocument();
    expect(screen.getByRole('searchbox')).toHaveValue('');
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
  });

  it('loads exactly once, filters locally and provides no Copy/Export/link actions', async () => {
    mocks.list.mockResolvedValue(snapshot());
    const storage = vi.spyOn(Storage.prototype, 'setItem');
    render(<LogsWorkspace host={host} />);
    expect(await screen.findByText('synthetic journal row')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledExactlyOnceWith(host.id, A);
    expect(screen.getByText('Info')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Filter this snapshot' }), 'no-match');
    expect(screen.getByText('No entries match this filter.')).toBeInTheDocument();
    await userEvent.clear(screen.getByRole('searchbox'));
    expect(screen.getByText('synthetic journal row')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('button', { name: /copy|export/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(storage).not.toHaveBeenCalled();
  });

  it('does not poll or refetch on focus during 65 seconds', async () => {
    vi.useFakeTimers();
    mocks.list.mockResolvedValue(snapshot());
    await act(async () => { render(<LogsWorkspace host={host} />); });
    expect(screen.getByText('synthetic journal row')).toBeInTheDocument();
    await act(async () => { await vi.advanceTimersByTimeAsync(65_000); window.dispatchEvent(new Event('focus')); });
    expect(mocks.list).toHaveBeenCalledTimes(1);
  });

  it('refreshes exactly once despite repeated clicks and replaces the snapshot', async () => {
    const next = deferred<SystemJournalSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(next.promise);
    render(<LogsWorkspace host={host} />);
    await screen.findByText('synthetic journal row');
    const button = screen.getByRole('button', { name: 'Refresh' });
    fireEvent.click(button); fireEvent.click(button);
    expect(button).toBeDisabled();
    expect(mocks.list).toHaveBeenCalledTimes(2);
    await act(async () => { next.resolve(snapshot(host.id, A, 'fresh row')); });
    expect(screen.getByText('fresh row')).toBeInTheDocument();
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    expect(button).toBeEnabled();
  });

  it('keeps only the same-session snapshot after refresh failure with explicit stale wording', async () => {
    mocks.list.mockResolvedValueOnce(snapshot()).mockRejectedValueOnce(new Error('private remote diagnostics'));
    render(<LogsWorkspace host={host} />);
    await screen.findByText('synthetic journal row');
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText('Refresh failed — showing the last successful snapshot.')).toBeInTheDocument();
    expect(screen.getByText('synthetic journal row')).toBeInTheDocument();
    expect(screen.getByText('2026-09-27T10:00:00Z')).toBeInTheDocument();
    expect(screen.queryByText('private remote diagnostics')).not.toBeInTheDocument();
  });

  it('uses permission-dependent empty wording', async () => {
    mocks.list.mockResolvedValue({ ...snapshot(), entries: [] });
    render(<LogsWorkspace host={host} />);
    expect(await screen.findByText('No accessible system journal entries were returned.')).toBeInTheDocument();
    expect(screen.queryByText('This host has no logs.')).not.toBeInTheDocument();
  });

  it('shows only fixed safe initial failure text and allows manual recovery', async () => {
    mocks.list.mockRejectedValueOnce(new Error('private remote diagnostic')).mockResolvedValueOnce(snapshot());
    render(<LogsWorkspace host={host} />);
    expect(await screen.findByText('System journal entries are unavailable for this connection.')).toBeInTheDocument();
    expect(screen.queryByText('private remote diagnostic')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await screen.findByText('synthetic journal row')).toBeInTheDocument();
  });

  it('immediately removes visible rows on disconnect and never reuses them on reconnect', async () => {
    const next = deferred<SystemJournalSnapshot>();
    mocks.list.mockResolvedValueOnce(snapshot()).mockReturnValueOnce(next.promise);
    const view = render(<LogsWorkspace host={host} />);
    await screen.findByText('synthetic journal row');
    mocks.sessionQuery.data = session(host.id, null, 'disconnected'); view.rerender(<LogsWorkspace host={host} />);
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    mocks.sessionQuery.data = session(host.id, B); view.rerender(<LogsWorkspace host={host} />);
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    await act(async () => { next.reject(new Error('failure')); });
    expect(screen.getByText('System journal entries are unavailable for this connection.')).toBeInTheDocument();
    expect(screen.queryByText(/last successful snapshot/)).not.toBeInTheDocument();
  });

  it('ignores delayed old-session success after replacement', async () => {
    const old = deferred<SystemJournalSnapshot>();
    const next = deferred<SystemJournalSnapshot>();
    mocks.list.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    const view = render(<LogsWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(1));
    mocks.sessionQuery.data = session(host.id, B); view.rerender(<LogsWorkspace host={host} />);
    await act(async () => { old.resolve(snapshot()); });
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    await act(async () => { next.resolve(snapshot(host.id, B, 'new-session row')); });
    expect(screen.getByText('new-session row')).toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(host.id, B);
  });

  it('isolates host switches and ignores delayed unmounted results and failures', async () => {
    const old = deferred<SystemJournalSnapshot>();
    const next = deferred<SystemJournalSnapshot>();
    mocks.list.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    const view = render(<LogsWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(1));
    mocks.sessionQuery.data = session(otherHost.id);
    view.rerender(<LogsWorkspace host={otherHost} />);
    await act(async () => { old.resolve(snapshot()); });
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
    expect(mocks.list).toHaveBeenLastCalledWith(otherHost.id, A);
    view.unmount();
    await act(async () => { next.reject(new Error('ignored')); });
    const late = deferred<SystemJournalSnapshot>();
    mocks.list.mockReturnValueOnce(late.promise);
    mocks.sessionQuery.data = session();
    const final = render(<LogsWorkspace host={host} />);
    await waitFor(() => expect(mocks.list).toHaveBeenCalledTimes(3));
    final.unmount();
    await act(async () => { late.resolve(snapshot()); });
  });

  it.each([[otherHost.id, A], [host.id, B]])('rejects DTO identity mismatch %s/%s', async (hostId, sessionId) => {
    mocks.list.mockResolvedValue(snapshot(hostId, sessionId));
    render(<LogsWorkspace host={host} />);
    expect(await screen.findByText('System journal entries are unavailable for this connection.')).toBeInTheDocument();
    expect(screen.queryByText('synthetic journal row')).not.toBeInTheDocument();
  });

  it('renders all priorities, missing/omitted states and inert hostile-looking text', async () => {
    const result = snapshot();
    const entry = result.entries[0];
    if (!entry) throw new Error('Missing synthetic test entry');
    const hostileText = '<img src=x onerror=alert(1)> https://example.test \\u{001b}[31m\\u{202e}\\n';
    const labels = ['Emergency', 'Alert', 'Critical', 'Error', 'Warning', 'Notice', 'Info', 'Debug'];
    result.entries = labels.map((label, index) => ({ ...entry, priority: label.toLowerCase() as JournalPriority, message: index === 0 ? hostileText : `row ${index}` }));
    result.entries.push({ ...entry, priority: null, message: null, messageState: 'missing' });
    result.entries.push({ ...entry, priority: null, message: null, messageState: 'omitted' });
    mocks.list.mockResolvedValue(result);
    const { container } = render(<LogsWorkspace host={host} />);
    expect(await screen.findByText(hostileText)).toBeInTheDocument();
    for (const label of labels) expect(screen.getByText(label)).toBeInTheDocument();
    expect(screen.getByText('Message missing')).toBeInTheDocument();
    expect(screen.getByText('Message omitted')).toBeInTheDocument();
    expect(container.querySelector('img, a')).toBeNull();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled());
  });
});
