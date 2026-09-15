import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Host, TerminalSession } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { applicationError, terminalApi } from '../../api/client';
import { useHostSession } from '../../api/queries';
import { ConnectionBadge } from '../../components/ConnectionBadge';
import { Icon } from '../../components/Icon';
import { TerminalPane } from './TerminalPane';
import { mergeTerminalSession, type SessionUpdateSource } from './terminalFlow';

type SearchRequest = {
  text: string;
  direction: 'next' | 'previous';
  nonce: number;
};

export function TerminalWorkspace({
  host,
  visible,
  onShowOverview,
}: {
  host: Host;
  visible: boolean;
  onShowOverview: () => void;
}) {
  const hostSession = useHostSession(host.id);
  const [sessions, setSessions] = useState<TerminalSession[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchText, setSearchText] = useState('');
  const [searchRequest, setSearchRequest] = useState<SearchRequest | null>(null);
  const mutationRevisions = useRef(new Map<string, number>());

  const updateSession = useCallback((next: TerminalSession, source: SessionUpdateSource) => {
    setSessions((current) => {
      let changed = false;
      const updated = current.map((item) => {
        if (item.id !== next.id) return item;
        const merged = mergeTerminalSession(item, next, source);
        changed ||= merged !== item;
        return merged;
      });
      return changed ? updated : current;
    });
  }, []);
  const acceptPolledSession = useCallback(
    (next: TerminalSession) => updateSession(next, 'poll'),
    [updateSession],
  );
  const reportError = useCallback((message: string | null) => setError(message), []);

  useEffect(() => {
    let cancelled = false;
    async function refresh() {
      try {
        const next = await terminalApi.list(host.id);
        if (cancelled) return;
        setSessions(next);
        setActiveId((current) =>
          current && next.some((session) => session.id === current) ? current : (next[0]?.id ?? null),
        );
        setError(null);
      } catch (reason) {
        if (!cancelled) setError(applicationError(reason).message);
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void refresh();
    return () => {
      cancelled = true;
    };
  }, [host.id]);

  const activeSession = useMemo(
    () => sessions.find((session) => session.id === activeId) ?? null,
    [activeId, sessions],
  );
  const connected = hostSession.data?.state === 'connected';

  async function openTerminal() {
    if (!connected || opening) return;
    setOpening(true);
    setError(null);
    try {
      const terminal = await terminalApi.open(host.id, {
        columns: 100,
        rows: 30,
        pixelWidth: 0,
        pixelHeight: 0,
      });
      setSessions((current) => [...current, terminal]);
      setActiveId(terminal.id);
    } catch (reason) {
      setError(applicationError(reason).message);
    } finally {
      setOpening(false);
    }
  }

  async function closeTerminal(session: TerminalSession) {
    mutationRevisions.current.set(session.id, (mutationRevisions.current.get(session.id) ?? 0) + 1);
    setError(null);
    try {
      await terminalApi.close(session);
      setSessions((current) => {
        const remaining = current.filter((item) => item.id !== session.id);
        setActiveId((active) => (active === session.id ? (remaining[0]?.id ?? null) : active));
        return remaining;
      });
    } catch (reason) {
      setError(applicationError(reason).message);
    }
  }

  async function commitRename(session: TerminalSession) {
    const revision = (mutationRevisions.current.get(session.id) ?? 0) + 1;
    mutationRevisions.current.set(session.id, revision);
    try {
      const renamed = await terminalApi.rename(session, renameValue);
      if (mutationRevisions.current.get(session.id) === revision) updateSession(renamed, 'rename');
      setRenamingId(null);
    } catch (reason) {
      setError(applicationError(reason).message);
    }
  }

  function search(direction: 'next' | 'previous') {
    if (!searchText) return;
    setSearchRequest((current) => ({ text: searchText, direction, nonce: (current?.nonce ?? 0) + 1 }));
  }

  function selectRelative(direction: -1 | 1) {
    if (!sessions.length) return;
    const current = Math.max(0, sessions.findIndex((session) => session.id === activeId));
    const next = (current + direction + sessions.length) % sessions.length;
    setActiveId(sessions[next]?.id ?? null);
  }

  return (
    <section className="terminal-workspace" aria-label={`${host.displayName} terminal workspace`}>
      <header className="terminal-heading">
        <div>
          <div className="eyebrow">REMOTE TERMINAL</div>
          <div className="terminal-title-line">
            <h1>{host.displayName}</h1>
            <ConnectionBadge state={hostSession.data?.state ?? 'disconnected'} />
          </div>
          <p className="mono">
            {host.connection.username}@{host.connection.hostname}:{host.connection.port}
          </p>
        </div>
        <div className="heading-actions">
          <Button variant="ghost" onClick={onShowOverview}>
            Overview
          </Button>
          <Button variant="primary" disabled={!connected || opening} onClick={() => void openTerminal()}>
            <Icon name="plus" size={15} />
            {opening ? 'Opening…' : 'New terminal'}
          </Button>
        </div>
      </header>

      <div className="terminal-tabs-row">
        <div className="terminal-tabs" role="tablist" aria-label="Terminal sessions">
          {sessions.map((session) =>
            renamingId === session.id ? (
              <form
                key={session.id}
                className="terminal-rename"
                onSubmit={(event) => {
                  event.preventDefault();
                  void commitRename(session);
                }}
              >
                <label className="sr-only" htmlFor={`rename-${session.id}`}>
                  Terminal name
                </label>
                <input
                  id={`rename-${session.id}`}
                  value={renameValue}
                  maxLength={64}
                  autoFocus
                  onChange={(event) => setRenameValue(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape') setRenamingId(null);
                  }}
                  onBlur={() => setRenamingId(null)}
                />
              </form>
            ) : (
              <button
                key={session.id}
                role="tab"
                aria-selected={session.id === activeId}
                className={`terminal-tab ${session.id === activeId ? 'terminal-tab--active' : ''}`}
                onClick={() => setActiveId(session.id)}
                onDoubleClick={() => {
                  setRenameValue(session.label);
                  setRenamingId(session.id);
                }}
                onKeyDown={(event) => {
                  if (event.key === 'ArrowLeft') selectRelative(-1);
                  if (event.key === 'ArrowRight') selectRelative(1);
                }}
              >
                <span className={`terminal-state terminal-state--${session.state}`} aria-hidden="true" />
                {session.label}
              </button>
            ),
          )}
        </div>
        {activeSession && (
          <div className="terminal-tab-actions">
            <button
              aria-label="Rename active terminal"
              title="Rename terminal"
              onClick={() => {
                setRenameValue(activeSession.label);
                setRenamingId(activeSession.id);
              }}
            >
              Rename
            </button>
            <button aria-label="Search terminal" title="Search (Ctrl+Shift+F)" onClick={() => setSearchOpen(true)}>
              Search
            </button>
            <button
              aria-label="Close active terminal"
              title="Close terminal (Ctrl+Shift+W)"
              onClick={() => void closeTerminal(activeSession)}
            >
              Close
            </button>
          </div>
        )}
      </div>

      {searchOpen && activeSession && (
        <form
          className="terminal-search"
          role="search"
          onSubmit={(event) => {
            event.preventDefault();
            search('next');
          }}
        >
          <label className="sr-only" htmlFor={`terminal-search-${host.id}`}>
            Search terminal scrollback
          </label>
          <input
            id={`terminal-search-${host.id}`}
            value={searchText}
            autoFocus
            placeholder="Search terminal output"
            onChange={(event) => {
              setSearchText(event.target.value);
              setSearchRequest({ text: event.target.value, direction: 'next', nonce: Date.now() });
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') setSearchOpen(false);
            }}
          />
          <button type="button" onClick={() => search('previous')} aria-label="Previous match">
            Previous
          </button>
          <button type="submit" aria-label="Next match">
            Next
          </button>
          <button
            type="button"
            aria-label="Close search"
            onClick={() => {
              setSearchOpen(false);
              setSearchText('');
              setSearchRequest(null);
            }}
          >
            ×
          </button>
        </form>
      )}

      {error && (
        <Notice>
          <strong>Terminal request failed.</strong> {error}
        </Notice>
      )}
      {loading ? (
        <div className="terminal-empty">
          <Spinner label="Loading terminal workspace…" />
        </div>
      ) : sessions.length === 0 ? (
        <div className="terminal-empty">
          <span className="large-icon"><Icon name="terminal" size={30} /></span>
          <h2>{connected ? 'Open a remote shell' : 'Connect this host first'}</h2>
          <p>
            {connected
              ? 'Each tab is an independent SSH PTY. Terminal contents stay in memory and are never saved by NexusOps.'
              : 'Open Overview to verify and connect this host, then return here to start a terminal.'}
          </p>
          {connected ? (
            <Button variant="primary" onClick={() => void openTerminal()}>Open terminal</Button>
          ) : (
            <Button onClick={onShowOverview}>Open overview</Button>
          )}
        </div>
      ) : (
        <div className="terminal-stage">
          {sessions.map((session) => (
            <TerminalPane
              key={session.id}
              session={session}
              active={session.id === activeId}
              visible={visible}
              search={session.id === activeId ? searchRequest : null}
              onSession={acceptPolledSession}
              onError={reportError}
              onOpenSearch={() => setSearchOpen(true)}
              onNew={() => void openTerminal()}
              onClose={() => void closeTerminal(session)}
            />
          ))}
        </div>
      )}
      <div className="terminal-shortcuts" aria-label="Terminal shortcut help">
        <span>Ctrl+Shift+C Copy</span>
        <span>Ctrl+Shift+V Paste</span>
        <span>Ctrl+Shift+F Search</span>
        <span>Ctrl+Shift+T New</span>
        <span>Ctrl+Shift+W Close</span>
      </div>
    </section>
  );
}
