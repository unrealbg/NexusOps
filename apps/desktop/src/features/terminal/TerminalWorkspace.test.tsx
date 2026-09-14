import { render, screen, waitFor, within } from '@testing-library/react';
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
      return Promise.resolve(terminal(hostId, `terminal-${mocks.openCount}`, `Terminal ${mocks.openCount}`));
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
    onData() {
      return { dispose() {} };
    }
    onBinary() {
      return { dispose() {} };
    }
    attachCustomKeyEventHandler() {}
    write() {}
    focus = mocks.focus;
    clear() {}
    dispose() {}
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
  mocks.poll.mockReset().mockImplementation((session: TerminalSession) =>
    Promise.resolve({ session, chunksBase64: [] }),
  );
  mocks.write.mockReset().mockResolvedValue(undefined);
  mocks.resize.mockReset().mockResolvedValue(undefined);
  mocks.close.mockReset().mockResolvedValue(undefined);
  mocks.rename.mockReset().mockImplementation((session: TerminalSession, label: string) =>
    Promise.resolve({ ...session, label }),
  );
  mocks.searchNext.mockReset();
  mocks.searchPrevious.mockReset();
  mocks.focus.mockReset();
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
});
