import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Host, TerminalSession } from '@nexusops/protocol';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  hostState: 'connected',
  sessions: new Map<string, TerminalSession[]>(),
  openCount: 0,
  poll: vi.fn(),
  write: vi.fn(),
  resize: vi.fn(),
  close: vi.fn(),
  rename: vi.fn(),
  searchNext: vi.fn(),
  searchPrevious: vi.fn(),
  focus: vi.fn(),
  xtermWrites: [] as Uint8Array[],
  xtermWriteCallbacks: [] as Array<() => void>,
  xtermDataHandlers: [] as Array<(data: string) => void>,
  xtermDisposals: 0,
}));

vi.mock('../../api/queries', () => ({
  useHostSession: () => ({ data: { state: mocks.hostState } }),
}));

vi.mock('../../api/client', () => ({
  applicationError: (error: unknown) =>
    typeof error === 'object' && error !== null && 'message' in error
      ? error
      : { code: 'connection', message: 'failed', hostKey: null },
  terminalApi: {
    list: vi.fn((hostId: string) => Promise.resolve(mocks.sessions.get(hostId) ?? [])),
    open: vi.fn((hostId: string) => {
      mocks.openCount += 1;
      const opened = terminal(hostId, `terminal-${mocks.openCount}`, `Terminal ${mocks.openCount}`);
      mocks.sessions.set(hostId, [...(mocks.sessions.get(hostId) ?? []), opened]);
      return Promise.resolve(opened);
    }),
    poll: mocks.poll,
    write: mocks.write,
    resize: mocks.resize,
    close: mocks.close,
    rename: mocks.rename,
  },
}));

vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  readText: vi.fn(() => Promise.resolve('clipboard')),
  writeText: vi.fn(() => Promise.resolve()),
}));

vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit() {}
    dispose() {}
  },
}));

vi.mock('@xterm/addon-search', () => ({
  SearchAddon: class {
    findNext = mocks.searchNext;
    findPrevious = mocks.searchPrevious;
    clearDecorations() {}
    dispose() {}
  },
}));

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols: number;
    rows: number;
    constructor(options: { cols: number; rows: number }) {
      this.cols = options.cols;
      this.rows = options.rows;
    }
    loadAddon() {}
    open(element: HTMLElement) {
      const terminal = document.createElement('div');
      terminal.className = 'xterm';
      const textarea = document.createElement('textarea');
      terminal.append(textarea);
      element.append(terminal);
    }
    onData(handler: (data: string) => void) {
      mocks.xtermDataHandlers.push(handler);
      return { dispose() {} };
    }
    onBinary() {
      return { dispose() {} };
    }
    attachCustomKeyEventHandler() {}
    write(data: Uint8Array, callback: () => void) {
      mocks.xtermWrites.push(data.slice());
      mocks.xtermWriteCallbacks.push(callback);
    }
    focus = mocks.focus;
    clear() {}
    dispose() {
      mocks.xtermDisposals += 1;
    }
    hasSelection() {
      return false;
    }
    getSelection() {
      return '';
    }
    paste() {}
    selectAll() {}
  },
}));

import { TerminalWorkspace } from './TerminalWorkspace';
import { bytesToBase64 } from './terminalFlow';

const host = (id: string, name: string): Host => ({
  id,
  displayName: name,
  connection: { hostname: `${id}.test`, port: 2222, username: 'operator', authentication: 'password' },
});

function terminal(hostId: string, id: string, label: string, state: TerminalSession['state'] = 'open'): TerminalSession {
  return {
    id,
    hostId,
    hostSessionId: `${hostId}-connection`,
    label,
    state,
    size: { columns: 80, rows: 24, pixelWidth: 0, pixelHeight: 0 },
    error: null,
  };
}

beforeEach(() => {
  mocks.hostState = 'connected';
  mocks.sessions.clear();
  mocks.openCount = 0;
  mocks.poll.mockReset().mockImplementation((session: TerminalSession) => {
    const backend = (mocks.sessions.get(session.hostId) ?? []).find((item) => item.id === session.id);
    if (!backend) return Promise.reject({ code: 'notFound', message: 'removed', hostKey: null });
    return Promise.resolve({
      session: { ...backend },
      chunksBase64: [],
      outputDrained: ['closed', 'failed', 'disconnected'].includes(backend.state),
    });
  });
  mocks.write.mockReset().mockResolvedValue(undefined);
  mocks.resize.mockReset().mockResolvedValue(undefined);
  mocks.close.mockReset().mockImplementation((session: TerminalSession) => {
    mocks.sessions.set(
      session.hostId,
      (mocks.sessions.get(session.hostId) ?? []).filter((item) => item.id !== session.id),
    );
    return Promise.resolve();
  });
  mocks.rename.mockReset().mockImplementation((session: TerminalSession, label: string) => {
    const renamed = { ...session, label };
    mocks.sessions.set(
      session.hostId,
      (mocks.sessions.get(session.hostId) ?? []).map((item) => (item.id === session.id ? renamed : item)),
    );
    return Promise.resolve(renamed);
  });
  mocks.searchNext.mockReset();
  mocks.searchPrevious.mockReset();
  mocks.focus.mockReset();
  mocks.xtermWrites.length = 0;
  mocks.xtermWriteCallbacks.length = 0;
  mocks.xtermDataHandlers.length = 0;
  mocks.xtermDisposals = 0;
});

describe('TerminalWorkspace', () => {
  test('creates, switches, renames, and closes distinct terminal tabs', async () => {
    const user = userEvent.setup();
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByText('Open a remote shell');

    await user.click(screen.getByRole('button', { name: 'Open terminal' }));
    const first = await screen.findByRole('tab', { name: /Terminal 1/ });
    await user.click(screen.getByRole('button', { name: 'New terminal' }));
    const second = await screen.findByRole('tab', { name: /Terminal 2/ });
    expect(second).toHaveAttribute('aria-selected', 'true');

    await user.click(first);
    expect(first).toHaveAttribute('aria-selected', 'true');
    await user.click(screen.getByRole('button', { name: 'Rename active terminal' }));
    const name = screen.getByLabelText('Terminal name');
    await user.clear(name);
    await user.type(name, 'Production shell{Enter}');
    expect(await screen.findByRole('tab', { name: /Production shell/ })).toBeVisible();

    await user.click(screen.getByRole('button', { name: 'Close active terminal' }));
    await waitFor(() => expect(screen.queryByRole('tab', { name: /Production shell/ })).toBeNull());
    expect(screen.getByRole('tab', { name: /Terminal 2/ })).toBeVisible();
    expect(mocks.close).toHaveBeenCalledTimes(1);
  });

  test('keeps host session mappings isolated across mounted workspaces', async () => {
    const alpha = terminal('host-a', 'a-terminal', 'Alpha shell');
    const beta = terminal('host-b', 'b-terminal', 'Beta shell');
    mocks.sessions.set('host-a', [alpha]);
    mocks.sessions.set('host-b', [beta]);
    render(
      <>
        <TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />
        <TerminalWorkspace host={host('host-b', 'Beta')} visible={false} onShowOverview={() => {}} />
      </>,
    );
    const alphaWorkspace = await screen.findByRole('region', { name: 'Alpha terminal workspace' });
    const betaWorkspace = await screen.findByRole('region', { name: 'Beta terminal workspace' });
    expect(within(alphaWorkspace).getByRole('tab', { name: /Alpha shell/ })).toBeVisible();
    expect(within(alphaWorkspace).queryByText('Beta shell')).toBeNull();
    expect(within(betaWorkspace).getByRole('tab', { name: /Beta shell/ })).toBeVisible();
    expect(within(betaWorkspace).queryByText('Alpha shell')).toBeNull();
    expect(mocks.poll).toHaveBeenCalledWith(expect.objectContaining({ hostId: 'host-a', id: 'a-terminal' }));
    expect(mocks.poll).toHaveBeenCalledWith(expect.objectContaining({ hostId: 'host-b', id: 'b-terminal' }));
  });

  test('shows ended state and performs scrollback search locally', async () => {
    const user = userEvent.setup();
    const ended = terminal('host-a', 'ended-terminal', 'Ended shell', 'disconnected');
    mocks.sessions.set('host-a', [ended]);
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    expect(await screen.findByText('Connection lost')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Search terminal' }));
    const search = screen.getByLabelText('Search terminal scrollback');
    await user.type(search, 'local-only');
    await user.click(screen.getByRole('button', { name: 'Next match' }));
    await user.click(screen.getByRole('button', { name: 'Previous match' }));
    expect(mocks.searchNext).toHaveBeenCalledWith('local-only', expect.any(Object));
    expect(mocks.searchPrevious).toHaveBeenCalledWith('local-only', expect.any(Object));
    expect(mocks.write).not.toHaveBeenCalled();
  });

  test('requires a connected host before opening a PTY', async () => {
    mocks.hostState = 'disconnected';
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    expect(await screen.findByText('Connect this host first')).toBeVisible();
    expect(screen.getByRole('button', { name: 'New terminal' })).toBeDisabled();
  });

  test('does not let a poll started before rename overwrite the newer label', async () => {
    const user = userEvent.setup();
    const original = terminal('host-a', 'terminal-a', 'Original shell');
    mocks.sessions.set('host-a', [original]);
    let resolvePoll!: (value: { session: TerminalSession; chunksBase64: string[]; outputDrained: boolean }) => void;
    mocks.poll.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolvePoll = resolve;
        }),
    );

    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    const tab = await screen.findByRole('tab', { name: /Original shell/ });
    await waitFor(() => expect(mocks.poll).toHaveBeenCalledTimes(1));
    await user.click(tab);
    await user.click(screen.getByRole('button', { name: 'Rename active terminal' }));
    const name = screen.getByLabelText('Terminal name');
    await user.clear(name);
    await user.type(name, 'Production shell{Enter}');
    expect(await screen.findByRole('tab', { name: /Production shell/ })).toBeVisible();

    resolvePoll({ session: original, chunksBase64: [], outputDrained: false });
    await waitFor(() => expect(screen.getByRole('tab', { name: /Production shell/ })).toBeVisible());
    expect(screen.queryByRole('tab', { name: /Original shell/ })).toBeNull();
  });

  test('ignores an older rename result that resolves after a newer rename', async () => {
    const user = userEvent.setup();
    const original = terminal('host-a', 'terminal-a', 'Original shell');
    mocks.sessions.set('host-a', [original]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    const resolvers: Array<(session: TerminalSession) => void> = [];
    mocks.rename.mockImplementation(
      (_session: TerminalSession, label: string) =>
        new Promise((resolve) => resolvers.push(() => resolve({ ...original, label }))),
    );

    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Original shell/ });
    await user.click(screen.getByRole('button', { name: 'Rename active terminal' }));
    const name = screen.getByLabelText('Terminal name');
    await user.clear(name);
    await user.type(name, 'Older result{Enter}');
    await user.clear(name);
    await user.type(name, 'Newest result{Enter}');
    expect(resolvers).toHaveLength(2);

    resolvers[1]?.({ ...original, label: 'Newest result' });
    expect(await screen.findByRole('tab', { name: /Newest result/ })).toBeVisible();
    resolvers[0]?.({ ...original, label: 'Older result' });
    await waitFor(() => expect(screen.getByRole('tab', { name: /Newest result/ })).toBeVisible());
    expect(screen.queryByRole('tab', { name: /Older result/ })).toBeNull();
  });

  test('keeps one output consumer across parent renders and drains a multi-chunk batch exactly once', async () => {
    const user = userEvent.setup();
    const original = terminal('host-a', 'terminal-a', 'Original shell');
    mocks.sessions.set('host-a', [original]);
    const marker = new TextEncoder().encode('REVIEW-02-FINAL-MARKER');
    const chunks = [65, 66, 67, 68].map((value) => new Uint8Array(16 * 1024).fill(value));
    chunks[3]?.set(marker, (chunks[3]?.length ?? 0) - marker.length);
    const secondPolls: Array<
      (value: { session: TerminalSession; chunksBase64: string[]; outputDrained: boolean }) => void
    > = [];
    mocks.poll
      .mockResolvedValueOnce({
        session: original,
        chunksBase64: chunks.map(bytesToBase64),
        outputDrained: false,
      })
      .mockImplementation(
        () =>
          new Promise((resolve) => {
            secondPolls.push(resolve);
          }),
      );

    const view = render(
      <TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />,
    );
    await waitFor(() => expect(mocks.xtermWrites).toHaveLength(1));

    await user.click(screen.getByRole('button', { name: 'Rename active terminal' }));
    await user.type(screen.getByLabelText('Terminal name'), ' updated');
    await user.keyboard('{Escape}');
    await user.click(screen.getByRole('button', { name: 'Search terminal' }));
    await user.type(screen.getByLabelText('Search terminal scrollback'), 'marker');

    expect(mocks.poll).toHaveBeenCalledTimes(1);
    expect(mocks.xtermDisposals).toBe(0);
    for (let index = 0; index < chunks.length; index += 1) {
      mocks.xtermWriteCallbacks.shift()?.();
      if (index + 1 < chunks.length) {
        await waitFor(() => expect(mocks.xtermWrites).toHaveLength(index + 2));
      }
    }
    await waitFor(() => expect(mocks.poll).toHaveBeenCalledTimes(2));
    expect(secondPolls).toHaveLength(1);
    secondPolls[0]?.({
      session: terminal('host-a', 'terminal-a', 'Original shell', 'closed'),
      chunksBase64: [],
      outputDrained: true,
    });
    expect(await screen.findByText('Connection lost')).toBeVisible();
    await new Promise((resolve) => window.setTimeout(resolve, 50));
    expect(mocks.poll).toHaveBeenCalledTimes(2);

    const expected = chunks.flatMap((bytes) => [...bytes]);
    const actual = mocks.xtermWrites.flatMap((bytes) => [...bytes]);
    expect(new Uint8Array(actual)).toEqual(new Uint8Array(expected));
    expect(new TextDecoder().decode(mocks.xtermWrites.at(-1))).toContain('REVIEW-02-FINAL-MARKER');
    expect(mocks.xtermDisposals).toBe(0);
    view.unmount();
    expect(mocks.xtermDisposals).toBe(1);
  });

  test('marks a removed open backend session failed and stops polling across parent renders', async () => {
    const user = userEvent.setup();
    const original = terminal('host-a', 'terminal-a', 'Original shell');
    mocks.sessions.set('host-a', [original]);
    mocks.poll.mockRejectedValue({
      code: 'notFound',
      message: 'The terminal session was removed.',
      hostKey: null,
    });

    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    expect(await screen.findByText('Terminal failed')).toBeVisible();
    await waitFor(() => expect(mocks.poll).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole('button', { name: 'Search terminal' }));
    await user.type(screen.getByLabelText('Search terminal scrollback'), 'still stopped');
    await new Promise((resolve) => window.setTimeout(resolve, 50));

    expect(screen.getAllByText('The terminal session was removed.').length).toBeGreaterThan(0);
    expect(mocks.poll).toHaveBeenCalledTimes(1);
  });

  test('stops terminal input visibly after an ambiguous write failure without replaying bytes', async () => {
    const original = terminal('host-a', 'terminal-a', 'Original shell');
    mocks.sessions.set('host-a', [original]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    mocks.write.mockRejectedValue({
      code: 'terminalStream',
      message: 'The terminal input acknowledgement failed.',
      hostKey: null,
    });

    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await waitFor(() => expect(mocks.xtermDataHandlers).toHaveLength(1));
    act(() => mocks.xtermDataHandlers[0]?.('FIRST'));
    await waitFor(() => expect(mocks.write).toHaveBeenCalledTimes(1));
    expect(
      await screen.findByText(/Terminal input stopped after a write failure because delivery may be incomplete/),
    ).toBeVisible();

    act(() => mocks.xtermDataHandlers[0]?.('SECOND'));
    await new Promise((resolve) => window.setTimeout(resolve, 25));
    expect(mocks.write).toHaveBeenCalledTimes(1);
  });
});
