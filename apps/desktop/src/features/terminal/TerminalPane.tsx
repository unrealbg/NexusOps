import { useEffect, useRef, useState } from 'react';
import type { TerminalSession, TerminalSize } from '@nexusops/protocol';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { terminalApi, applicationError } from '../../api/client';
import {
  BoundedTerminalInput,
  bytesToBase64,
  MAX_QUEUED_INPUT_BYTES,
  runTerminalPollLoop,
  TERMINAL_INPUT_FAILURE_MESSAGE,
} from './terminalFlow';

const RESIZE_DEBOUNCE_MS = 80;

type SearchRequest = {
  text: string;
  direction: 'next' | 'previous';
  nonce: number;
};

export function TerminalPane({
  session,
  active,
  visible,
  search,
  onSession,
  onError,
  onOpenSearch,
  onNew,
  onClose,
}: {
  session: TerminalSession;
  active: boolean;
  visible: boolean;
  search: SearchRequest | null;
  onSession: (session: TerminalSession) => void;
  onError: (message: string | null) => void;
  onOpenSearch: () => void;
  onNew: () => void;
  onClose: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const sessionRef = useRef(session);
  const inputQueue = useRef(new BoundedTerminalInput());
  const inputAbort = useRef(new AbortController());
  const inputTimer = useRef<number | null>(null);
  const flushingInput = useRef(false);
  const stopped = useRef(false);
  const lastSize = useRef<TerminalSize | null>(null);
  const resizeTimer = useRef<number | null>(null);
  const contextActionTaken = useRef(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; canCopy: boolean } | null>(null);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  async function flushInput() {
    if (flushingInput.current || stopped.current) return;
    flushingInput.current = true;
    try {
      await inputQueue.current.flush(
        (next) => terminalApi.write(sessionRef.current, bytesToBase64(next)),
        inputAbort.current.signal,
      );
    } catch (error) {
      if (!inputAbort.current.signal.aborted) {
        const reason = applicationError(error);
        onError(`${TERMINAL_INPUT_FAILURE_MESSAGE} ${reason.message}`);
      }
    } finally {
      flushingInput.current = false;
    }
  }

  function queueInput(bytes: Uint8Array) {
    if (sessionRef.current.state !== 'open' || stopped.current) return;
    if (!inputQueue.current.enqueue(bytes)) {
      if (inputQueue.current.failed) {
        onError(TERMINAL_INPUT_FAILURE_MESSAGE);
        return;
      }
      onError(
        `Terminal input was rejected because the ${MAX_QUEUED_INPUT_BYTES / 1024} KiB send queue is full.`,
      );
      return;
    }
    if (inputTimer.current !== null) return;
    inputTimer.current = window.setTimeout(() => {
      inputTimer.current = null;
      void flushInput();
    }, 8);
  }

  async function copySelection() {
    const terminal = terminalRef.current;
    if (!terminal?.hasSelection()) return;
    await writeText(terminal.getSelection());
  }

  async function pasteClipboard() {
    const terminal = terminalRef.current;
    if (!terminal || sessionRef.current.state !== 'open') return;
    const text = await readText();
    if (text) terminal.paste(text);
  }

  function runContextAction(action: () => void, focusTerminal = true) {
    if (contextActionTaken.current) return;
    contextActionTaken.current = true;
    setContextMenu(null);
    action();
    if (focusTerminal) terminalRef.current?.focus();
  }

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    stopped.current = false;
    inputQueue.current = new BoundedTerminalInput();
    inputAbort.current = new AbortController();
    const terminal = new Terminal({
      cols: session.size.columns,
      rows: session.size.rows,
      convertEol: false,
      cursorBlink: true,
      cursorStyle: 'block',
      fontFamily: "'Cascadia Mono', 'Cascadia Code', 'SFMono-Regular', Consolas, monospace",
      fontSize: 13,
      lineHeight: 1.16,
      letterSpacing: 0,
      minimumContrastRatio: 4.5,
      screenReaderMode: true,
      scrollback: 10_000,
      theme: {
        background: '#0b0f12',
        foreground: '#d7e0df',
        cursor: '#78d6b0',
        cursorAccent: '#0b0f12',
        selectionBackground: '#31584f',
        black: '#11171a',
        red: '#ef7d7d',
        green: '#78d6b0',
        yellow: '#e5c178',
        blue: '#77a7e8',
        magenta: '#c99be8',
        cyan: '#72cbd1',
        white: '#d7e0df',
        brightBlack: '#657179',
        brightRed: '#ff9a9a',
        brightGreen: '#9ce7c8',
        brightYellow: '#f1d596',
        brightBlue: '#9bbff0',
        brightMagenta: '#dfb9f2',
        brightCyan: '#9ce0e3',
        brightWhite: '#f4f7f6',
      },
    });
    const fit = new FitAddon();
    const searchAddon = new SearchAddon();
    terminal.loadAddon(fit);
    terminal.loadAddon(searchAddon);
    terminal.open(element);
    terminalRef.current = terminal;
    fitRef.current = fit;
    searchRef.current = searchAddon;

    const dataDisposable = terminal.onData((data) => queueInput(new TextEncoder().encode(data)));
    const binaryDisposable = terminal.onBinary((data) =>
      queueInput(Uint8Array.from(data, (character) => character.charCodeAt(0) & 0xff)),
    );
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown' || !event.ctrlKey || !event.shiftKey) return true;
      const key = event.key.toLowerCase();
      if (key === 'c') {
        event.preventDefault();
        void copySelection().catch(() => onError('The selected text could not be copied.'));
        return false;
      }
      if (key === 'v') {
        event.preventDefault();
        void pasteClipboard().catch(() => onError('Clipboard text could not be pasted.'));
        return false;
      }
      if (key === 'f') {
        event.preventDefault();
        onOpenSearch();
        return false;
      }
      if (key === 'k') {
        event.preventDefault();
        terminal.clear();
        return false;
      }
      if (key === 't') {
        event.preventDefault();
        onNew();
        return false;
      }
      if (key === 'w') {
        event.preventDefault();
        onClose();
        return false;
      }
      return true;
    });

    const contextHandler = (event: MouseEvent) => {
      event.preventDefault();
      const bounds = element.getBoundingClientRect();
      contextActionTaken.current = false;
      setContextMenu({
        x: Math.min(Math.max(8, event.clientX - bounds.left), Math.max(8, bounds.width - 150)),
        y: Math.min(Math.max(8, event.clientY - bounds.top), Math.max(8, bounds.height - 190)),
        canCopy: terminal.hasSelection(),
      });
    };
    element.addEventListener('contextmenu', contextHandler);

    return () => {
      stopped.current = true;
      inputAbort.current.abort();
      inputQueue.current.stop();
      if (inputTimer.current !== null) window.clearTimeout(inputTimer.current);
      if (resizeTimer.current !== null) window.clearTimeout(resizeTimer.current);
      element.removeEventListener('contextmenu', contextHandler);
      dataDisposable.dispose();
      binaryDisposable.dispose();
      searchAddon.dispose();
      fit.dispose();
      terminal.dispose();
      terminalRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
    };
    // A terminal emulator belongs to exactly one backend terminal ID.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.id]);

  useEffect(() => {
    const cancellation = new AbortController();
    const terminal = terminalRef.current;
    if (!terminal) return;
    void runTerminalPollLoop({
      getSession: () => sessionRef.current,
      poll: terminalApi.poll,
      terminal,
      onSession,
      onError: (reason) => onError(reason === null ? null : applicationError(reason).message),
      signal: cancellation.signal,
    });
    return () => {
      cancellation.abort();
    };
  }, [onError, onSession, session.id]);

  useEffect(() => {
    if (!visible || !active) return;
    const element = containerRef.current;
    const terminal = terminalRef.current;
    const fit = fitRef.current;
    if (!element || !terminal || !fit) return;
    const resize = () => {
      if (element.clientWidth < 20 || element.clientHeight < 20) return;
      fit.fit();
      terminal.focus();
      if (sessionRef.current.state !== 'open') return;
      const size: TerminalSize = {
        columns: terminal.cols,
        rows: terminal.rows,
        pixelWidth: Math.round(element.clientWidth * window.devicePixelRatio),
        pixelHeight: Math.round(element.clientHeight * window.devicePixelRatio),
      };
      if (
        lastSize.current?.columns === size.columns &&
        lastSize.current?.rows === size.rows &&
        lastSize.current?.pixelWidth === size.pixelWidth &&
        lastSize.current?.pixelHeight === size.pixelHeight
      )
        return;
      lastSize.current = size;
      if (resizeTimer.current !== null) window.clearTimeout(resizeTimer.current);
      resizeTimer.current = window.setTimeout(() => {
        void terminalApi.resize(sessionRef.current, size).catch((error) =>
          onError(applicationError(error).message),
        );
      }, RESIZE_DEBOUNCE_MS);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    const frame = window.requestAnimationFrame(resize);
    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(frame);
    };
  }, [active, onError, session.id, visible]);

  useEffect(() => {
    if (!active || !searchRef.current) return;
    if (!search?.text) {
      searchRef.current.clearDecorations();
      return;
    }
    const options = { caseSensitive: false, incremental: false };
    if (search.direction === 'previous') searchRef.current.findPrevious(search.text, options);
    else searchRef.current.findNext(search.text, options);
  }, [active, search]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      event.stopPropagation();
      setContextMenu(null);
      terminalRef.current?.focus();
    };
    window.addEventListener('pointerdown', close, { once: true });
    window.addEventListener('keydown', closeOnEscape, true);
    return () => {
      window.removeEventListener('pointerdown', close);
      window.removeEventListener('keydown', closeOnEscape, true);
    };
  }, [contextMenu]);

  const ended = ['closed', 'failed', 'disconnected'].includes(session.state);
  return (
    <div className="terminal-pane" hidden={!active} aria-label={`${session.label} terminal`}>
      <div ref={containerRef} className="terminal-canvas" data-terminal-id={session.id} />
      {ended && (
        <div className="terminal-ended" role="status">
          <strong>{session.state === 'failed' ? 'Terminal failed' : 'Connection lost'}</strong>
          <span>{session.error?.message ?? 'This terminal session has ended.'}</span>
          <button onClick={onNew}>Open new terminal</button>
        </div>
      )}
      {contextMenu && (
        <div
          className="terminal-context-menu"
          role="menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <button
            role="menuitem"
            disabled={!contextMenu.canCopy}
            onClick={() => runContextAction(() => {
              void copySelection().catch(() => onError('The selected text could not be copied.'));
            })}
          >
            Copy
          </button>
          <button
            role="menuitem"
            disabled={session.state !== 'open'}
            onClick={() => runContextAction(() => {
              void pasteClipboard().catch(() => onError('Clipboard text could not be pasted.'));
            })}
          >
            Paste
          </button>
          <button role="menuitem" onClick={() => runContextAction(() => terminalRef.current?.selectAll())}>
            Select all
          </button>
          <button role="menuitem" onClick={() => runContextAction(() => terminalRef.current?.clear())}>
            Clear
          </button>
          <button role="menuitem" onClick={() => runContextAction(onOpenSearch, false)}>
            Search
          </button>
        </div>
      )}
    </div>
  );
}
