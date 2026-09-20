import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
  paste: vi.fn(),
  selectAll: vi.fn(),
  clear: vi.fn(),
  readText: vi.fn(),
  writeText: vi.fn(),
  selectionAvailable: false,
  xtermWrites: [] as Uint8Array[],
  xtermWriteCallbacks: [] as Array<() => void>,
  xtermDataHandlers: [] as Array<(data: string) => void>,
  xtermPassedKeys: [] as Array<{ key: string; code: string }>,
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
  readText: mocks.readText,
  writeText: mocks.writeText,
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
    textarea: HTMLTextAreaElement | null = null;
    keyHandler: ((event: KeyboardEvent) => boolean) | null = null;
    dataHandler: ((data: string) => void) | null = null;
    constructor(options: { cols: number; rows: number }) {
      this.cols = options.cols;
      this.rows = options.rows;
    }
    loadAddon() {}
    open(element: HTMLElement) {
      const terminal = document.createElement('div');
      terminal.className = 'xterm';
      const textarea = document.createElement('textarea');
      this.textarea = textarea;
      textarea.addEventListener('keydown', (event) => {
        if (this.keyHandler?.(event) === false) return;
        mocks.xtermPassedKeys.push({ key: event.key, code: event.code });
        if (event.key === 'Escape') {
          this.dataHandler?.('\x1b');
        }
      });
      terminal.append(textarea);
      element.append(terminal);
    }
    onData(handler: (data: string) => void) {
      this.dataHandler = handler;
      mocks.xtermDataHandlers.push(handler);
      return { dispose() {} };
    }
    onBinary() {
      return { dispose() {} };
    }
    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) {
      this.keyHandler = handler;
    }
    write(data: Uint8Array, callback: () => void) {
      mocks.xtermWrites.push(data.slice());
      mocks.xtermWriteCallbacks.push(callback);
    }
    focus() {
      mocks.focus();
      this.textarea?.focus();
    }
    clear() {
      mocks.clear();
    }
    dispose() {
      mocks.xtermDisposals += 1;
    }
    hasSelection() {
      return mocks.selectionAvailable;
    }
    getSelection() {
      return '[selection]';
    }
    paste(text: string) {
      mocks.paste(text);
    }
    selectAll() {
      mocks.selectAll();
    }
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
  mocks.paste.mockReset();
  mocks.selectAll.mockReset();
  mocks.clear.mockReset();
  mocks.readText.mockReset().mockResolvedValue('[clipboard]');
  mocks.writeText.mockReset().mockResolvedValue(undefined);
  mocks.selectionAvailable = false;
  mocks.xtermWrites.length = 0;
  mocks.xtermWriteCallbacks.length = 0;
  mocks.xtermDataHandlers.length = 0;
  mocks.xtermPassedKeys.length = 0;
  mocks.xtermDisposals = 0;
});

describe('TerminalWorkspace', () => {
  async function openContextMenu() {
    const input = await activeTerminalInput();
    const canvas = input.closest('.terminal-canvas');
    if (!canvas) throw new Error('Terminal canvas missing');
    fireEvent.contextMenu(canvas, { clientX: 40, clientY: 40 });
    return screen.getByRole('menu');
  }

  async function activeTerminalInput() {
    return waitFor(() => {
      const input = document.querySelector('.terminal-pane:not([hidden]) .xterm textarea');
      if (!(input instanceof HTMLTextAreaElement)) throw new Error('Active terminal input missing');
      return input;
    });
  }

  function terminalKeyDown(input: HTMLElement, options: KeyboardEventInit) {
    const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...options });
    fireEvent(input, event);
    return event;
  }

  const applicationShortcuts = [
    { code: 'KeyC', englishKey: 'c', cyrillicKey: 'с', action: 'copy' },
    { code: 'KeyV', englishKey: 'v', cyrillicKey: 'в', action: 'paste' },
    { code: 'KeyF', englishKey: 'f', cyrillicKey: 'ф', action: 'search' },
    { code: 'KeyK', englishKey: 'k', cyrillicKey: 'к', action: 'clear' },
    { code: 'KeyT', englishKey: 't', cyrillicKey: 'т', action: 'new' },
    { code: 'KeyW', englishKey: 'w', cyrillicKey: 'ш', action: 'close' },
  ] as const;

  test.each(applicationShortcuts.flatMap(({ code, englishKey, cyrillicKey, action }) => [
    { code, key: englishKey, layout: 'English', action },
    { code, key: cyrillicKey, layout: 'Cyrillic', action },
  ]))('Ctrl+Shift+$code performs $action once with $layout event.key', async ({ code, key, action }) => {
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Shell')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    mocks.selectionAvailable = true;
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    const input = await activeTerminalInput();
    input.focus();

    const event = terminalKeyDown(input, { key, code, ctrlKey: true, shiftKey: true });
    expect(event.defaultPrevented).toBe(true);
    expect(mocks.xtermPassedKeys).toHaveLength(0);
    if (action === 'copy') {
      await waitFor(() => expect(mocks.writeText).toHaveBeenCalledTimes(1));
      expect(mocks.writeText).toHaveBeenCalledWith('[selection]');
    } else if (action === 'paste') {
      await waitFor(() => expect(mocks.paste).toHaveBeenCalledTimes(1));
      expect(mocks.readText).toHaveBeenCalledTimes(1);
      expect(mocks.paste).toHaveBeenCalledWith('[clipboard]');
    } else if (action === 'search') {
      expect(screen.getByLabelText('Search terminal scrollback')).toHaveFocus();
    } else if (action === 'clear') {
      expect(mocks.clear).toHaveBeenCalledTimes(1);
    } else if (action === 'new') {
      await screen.findByRole('tab', { name: /Terminal 1/ });
      expect(mocks.openCount).toBe(1);
    } else {
      await waitFor(() => expect(mocks.close).toHaveBeenCalledTimes(1));
      expect(mocks.close).toHaveBeenCalledWith(expect.objectContaining({ id: 'terminal-a' }));
    }
    expect(mocks.write).not.toHaveBeenCalled();
  });

  test('leaves other terminal keys untouched, including Escape and AltGr', async () => {
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Shell')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    const input = await activeTerminalInput();
    input.focus();

    const ordinaryKeys: KeyboardEventInit[] = [
      { key: 'q', code: 'KeyQ', ctrlKey: true, shiftKey: true },
      { key: 't', code: 'KeyQ', ctrlKey: true, shiftKey: true },
      { key: 'т', code: 'KeyT', shiftKey: true },
      { key: 'т', code: 'KeyT', ctrlKey: true },
      { key: 't', code: 'KeyT', ctrlKey: true, shiftKey: true, altKey: true },
      { key: 't', code: 'KeyT', ctrlKey: true, shiftKey: true, metaKey: true },
      { key: 'Escape', code: 'Escape' },
    ];
    for (const options of ordinaryKeys) {
      expect(terminalKeyDown(input, options).defaultPrevented).toBe(false);
    }
    const altGraph = new KeyboardEvent('keydown', {
      key: 't', code: 'KeyT', ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true,
    });
    Object.defineProperty(altGraph, 'getModifierState', { value: (modifier: string) => modifier === 'AltGraph' });
    fireEvent(input, altGraph);
    expect(altGraph.defaultPrevented).toBe(false);
    expect(mocks.xtermPassedKeys).toEqual([
      ...ordinaryKeys.map(({ key, code }) => ({ key, code })),
      { key: 't', code: 'KeyT' },
    ]);
    const outside = document.createElement('button');
    document.body.append(outside);
    expect(terminalKeyDown(outside, { key: 't', code: 'KeyT', ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(false);
    outside.remove();
    expect(mocks.openCount).toBe(0);
    await waitFor(() => expect(mocks.write).toHaveBeenCalledTimes(1));
    expect(mocks.write).toHaveBeenCalledWith(expect.objectContaining({ id: 'terminal-a' }), bytesToBase64(new Uint8Array([27])));
  });

  test('does not paste from an ended terminal and preserves repeat handling', async () => {
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Ended shell', 'disconnected')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    const input = await activeTerminalInput();
    input.focus();

    expect(terminalKeyDown(input, { key: 'в', code: 'KeyV', ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(true);
    expect(mocks.readText).not.toHaveBeenCalled();
    expect(mocks.paste).not.toHaveBeenCalled();
    expect(mocks.write).not.toHaveBeenCalled();
    expect(terminalKeyDown(input, { key: 'к', code: 'KeyK', ctrlKey: true, shiftKey: true, repeat: true }).defaultPrevented).toBe(true);
    expect(mocks.clear).toHaveBeenCalledTimes(1);
    expect(terminalKeyDown(input, { key: 'к', code: 'KeyK', ctrlKey: true, shiftKey: true, repeat: true }).defaultPrevented).toBe(true);
    expect(mocks.clear).toHaveBeenCalledTimes(2);
  });

  test.each([
    { code: '', key: 't' },
    { code: 'Unidentified', key: 'T' },
  ])('uses a Latin fallback only when physical code is $code', async ({ code, key }) => {
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Shell')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    const input = await activeTerminalInput();

    expect(terminalKeyDown(input, { key, code, ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(true);
    await screen.findByRole('tab', { name: /Terminal 1/ });
    expect(mocks.openCount).toBe(1);
  });

  test('dismisses each enabled context action exactly once and moves focus to its destination', async () => {
    const user = userEvent.setup();
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Shell')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    mocks.selectionAvailable = true;
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Shell/ });
    const input = await activeTerminalInput();

    await openContextMenu();
    await user.dblClick(screen.getByRole('menuitem', { name: 'Copy' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(mocks.writeText).toHaveBeenCalledTimes(1);
    expect(input).toHaveFocus();

    await openContextMenu();
    await user.dblClick(screen.getByRole('menuitem', { name: 'Paste' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(mocks.readText).toHaveBeenCalledTimes(1);
    expect(mocks.paste).toHaveBeenCalledTimes(1);
    expect(input).toHaveFocus();

    await openContextMenu();
    await user.click(screen.getByRole('menuitem', { name: 'Select all' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(mocks.selectAll).toHaveBeenCalledTimes(1);
    expect(input).toHaveFocus();

    await openContextMenu();
    await user.click(screen.getByRole('menuitem', { name: 'Clear' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(mocks.clear).toHaveBeenCalledTimes(1);
    expect(input).toHaveFocus();

    await openContextMenu();
    await user.click(screen.getByRole('menuitem', { name: 'Search' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(screen.getByLabelText('Search terminal scrollback')).toHaveFocus();
    expect(mocks.write).not.toHaveBeenCalled();
  });

  test('dismisses failed clipboard actions without replay and leaves the error visible', async () => {
    const user = userEvent.setup();
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Shell')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    mocks.selectionAvailable = true;
    mocks.writeText.mockRejectedValueOnce(new Error('copy failed'));
    mocks.readText.mockRejectedValueOnce(new Error('paste failed'));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Shell/ });
    const input = await activeTerminalInput();

    await openContextMenu();
    await user.click(screen.getByRole('menuitem', { name: 'Copy' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(await screen.findByText('The selected text could not be copied.')).toBeVisible();
    expect(mocks.writeText).toHaveBeenCalledTimes(1);
    expect(input).toHaveFocus();

    await openContextMenu();
    await user.click(screen.getByRole('menuitem', { name: 'Paste' }));
    expect(screen.queryByRole('menu')).toBeNull();
    expect(await screen.findByText('Clipboard text could not be pasted.')).toBeVisible();
    expect(mocks.readText).toHaveBeenCalledTimes(1);
    expect(mocks.paste).not.toHaveBeenCalled();
    expect(input).toHaveFocus();
  });

  test('Escape closes only the open menu and outside pointerdown still dismisses it', async () => {
    const user = userEvent.setup();
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Shell')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Shell/ });
    const input = await activeTerminalInput();
    input.focus();

    await openContextMenu();
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).toBeNull();
    expect(mocks.write).not.toHaveBeenCalled();
    expect(input).toHaveFocus();

    await user.keyboard('{Escape}');
    await waitFor(() => expect(mocks.write).toHaveBeenCalledTimes(1));
    expect(mocks.write).toHaveBeenCalledWith(expect.any(Object), bytesToBase64(new Uint8Array([27])));

    await openContextMenu();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  test('releases A menu when keyboard shortcut opens B and sends Escape only to B', async () => {
    const user = userEvent.setup();
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Terminal A')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Terminal A/ });
    const inputA = await activeTerminalInput();
    inputA.focus();
    await openContextMenu();

    await user.keyboard('{Control>}{Shift>}t{/Shift}{/Control}');
    const tabB = await screen.findByRole('tab', { name: /Terminal 1/ });
    expect(tabB).toHaveAttribute('aria-selected', 'true');
    const inputB = await activeTerminalInput();
    expect(inputB).not.toBe(inputA);
    await user.tab();
    expect(inputB).toHaveFocus();

    await user.keyboard('{Escape}');
    await waitFor(() => expect(mocks.write).toHaveBeenCalledTimes(1));
    expect(mocks.write).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'terminal-1' }),
      bytesToBase64(new Uint8Array([27])),
    );
    expect(inputB).toHaveFocus();
    expect(document.querySelector('.terminal-context-menu')).toBeNull();

    const tabA = screen.getByRole('tab', { name: /Terminal A/ });
    for (let step = 0; step < 8 && document.activeElement !== tabA; step += 1) {
      await user.tab({ shift: true });
    }
    expect(tabA).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(tabA).toHaveAttribute('aria-selected', 'true');
    expect(document.querySelector('.terminal-context-menu')).toBeNull();
    expect(mocks.xtermDisposals).toBe(0);
  });

  test('releases menu listeners when the workspace becomes invisible', async () => {
    const user = userEvent.setup();
    mocks.sessions.set('host-a', [terminal('host-a', 'terminal-a', 'Terminal A')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    const view = render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Terminal A/ });
    await activeTerminalInput();
    await openContextMenu();

    view.rerender(<TerminalWorkspace host={host('host-a', 'Alpha')} visible={false} onShowOverview={() => {}} />);
    await waitFor(() => expect(document.querySelector('.terminal-context-menu')).toBeNull());
    const outside = document.createElement('button');
    document.body.append(outside);
    const outsideKeydown = vi.fn();
    outside.addEventListener('keydown', outsideKeydown);
    outside.focus();
    await user.keyboard('{Escape}');
    expect(outsideKeydown).toHaveBeenCalledTimes(1);
    expect(outside).toHaveFocus();
    expect(mocks.write).not.toHaveBeenCalled();
    outside.remove();

    view.rerender(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    expect(document.querySelector('.terminal-context-menu')).toBeNull();
  });

  test('disabled Copy and Paste do not run clipboard operations', async () => {
    const user = userEvent.setup();
    mocks.sessions.set('host-a', [terminal('host-a', 'ended-terminal', 'Ended shell', 'disconnected')]);
    mocks.poll.mockImplementation(() => new Promise(() => {}));
    render(<TerminalWorkspace host={host('host-a', 'Alpha')} visible onShowOverview={() => {}} />);
    await screen.findByRole('tab', { name: /Ended shell/ });
    await openContextMenu();
    const copy = screen.getByRole('menuitem', { name: 'Copy' });
    const paste = screen.getByRole('menuitem', { name: 'Paste' });
    expect(copy).toBeDisabled();
    expect(paste).toBeDisabled();
    await user.click(copy);
    await user.click(paste);
    expect(mocks.readText).not.toHaveBeenCalled();
    expect(mocks.writeText).not.toHaveBeenCalled();
    expect(screen.getByRole('menu')).toBeVisible();
  });

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
