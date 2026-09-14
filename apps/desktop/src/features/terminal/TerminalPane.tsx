import { useEffect, useRef, useState } from 'react';
import type { TerminalSession, TerminalSize } from '@nexusops/protocol';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { terminalApi, applicationError } from '../../api/client';

const INPUT_CHUNK_BYTES = 16 * 1024;
const RESIZE_DEBOUNCE_MS = 80;
const POLL_INTERVAL_MS = 20;

type SearchRequest = {
  text: string;
  direction: 'next' | 'previous';
  nonce: number;
};

function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 8_192) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 8_192));
  }
  return btoa(binary);
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

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
  const inputQueue = useRef<Uint8Array[]>([]);
  const inputTimer = useRef<number | null>(null);
  const flushingInput = useRef(false);
  const stopped = useRef(false);
  const lastSize = useRef<TerminalSize | null>(null);
  const resizeTimer = useRef<number | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; canCopy: boolean } | null>(null);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  async function flushInput() {
    if (flushingInput.current || stopped.current) return;
    flushingInput.current = true;
    try {
      while (inputQueue.current.length && !stopped.current) {
        const next = inputQueue.current.shift();
        if (!next) break;
        await terminalApi.write(sessionRef.current, bytesToBase64(next));
      }
    } catch (error) {
      inputQueue.current = [];
      onError(applicationError(error).message);
    } finally {
      flushingInput.current = false;
    }
  }

  function queueInput(bytes: Uint8Array) {
    if (sessionRef.current.state !== 'open' || stopped.current) return;
    for (let offset = 0; offset < bytes.length; offset += INPUT_CHUNK_BYTES) {
      inputQueue.current.push(bytes.slice(offset, offset + INPUT_CHUNK_BYTES));
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

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    stopped.current = false;
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
      setContextMenu({
        x: Math.min(Math.max(8, event.clientX - bounds.left), Math.max(8, bounds.width - 150)),
        y: Math.min(Math.max(8, event.clientY - bounds.top), Math.max(8, bounds.height - 190)),
        canCopy: terminal.hasSelection(),
      });
    };
    element.addEventListener('contextmenu', contextHandler);

    return () => {
      stopped.current = true;
      inputQueue.current = [];
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
    let cancelled = false;
    let timer = 0;
    async function poll() {
      if (cancelled) return;
      try {
        const batch = await terminalApi.poll(sessionRef.current);
        for (const chunk of batch.chunksBase64) terminalRef.current?.write(base64ToBytes(chunk));
        onSession(batch.session);
        if (['closed', 'failed', 'disconnected'].includes(batch.session.state)) return;
      } catch (error) {
        onError(applicationError(error).message);
      }
      timer = window.setTimeout(() => void poll(), POLL_INTERVAL_MS);
    }
    void poll();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
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
    window.addEventListener('pointerdown', close, { once: true });
    return () => window.removeEventListener('pointerdown', close);
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
            onClick={() => void copySelection().catch(() => onError('The selected text could not be copied.'))}
          >
            Copy
          </button>
          <button
            role="menuitem"
            disabled={session.state !== 'open'}
            onClick={() => void pasteClipboard().catch(() => onError('Clipboard text could not be pasted.'))}
          >
            Paste
          </button>
          <button role="menuitem" onClick={() => terminalRef.current?.selectAll()}>
            Select all
          </button>
          <button role="menuitem" onClick={() => terminalRef.current?.clear()}>
            Clear
          </button>
          <button role="menuitem" onClick={onOpenSearch}>
            Search
          </button>
        </div>
      )}
    </div>
  );
}
