import { useEffect, useMemo, useRef, useState } from 'react';
import type { Host, JournalPriority, SystemJournalSnapshot } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { logsApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

const priorities: Record<JournalPriority, string> = {
  emergency: 'Emergency', alert: 'Alert', critical: 'Critical', error: 'Error',
  warning: 'Warning', notice: 'Notice', info: 'Info', debug: 'Debug',
};
const unavailable = 'System journal entries are unavailable for this connection.';

export function LogsWorkspace({ host }: { host: Host }) {
  const sessionQuery = useHostSession(host.id);
  const session = !sessionQuery.isError && sessionQuery.data?.hostId === host.id ? sessionQuery.data : null;
  const sessionId = session?.state === 'connected' ? session.hostSessionId : null;
  // Removing/replacing this child releases all sensitive rows and request state.
  // Retained query data is not authority after an error; recovery needs a fresh read.
  return sessionId ? <ConnectedLogs key={`${host.id}:${sessionId}`} host={host} sessionId={sessionId} /> : (
    <section aria-labelledby="logs-title">
      <header className="page-heading">
        <div><div className="eyebrow">READ-ONLY JOURNAL</div><h1 id="logs-title">Logs</h1><p>{host.displayName}</p></div>
        <Button disabled>Refresh</Button>
      </header>
      <p>{sessionQuery.isPending && !sessionQuery.isError && !sessionQuery.data
        ? 'Reading connection state…'
        : !session || session.state === 'connected'
          ? 'Current SSH session state is unavailable.'
          : 'Connect this host to inspect recent system journal entries.'}</p>
    </section>
  );
}

function ConnectedLogs({ host, sessionId }: { host: Host; sessionId: string }) {
  const [snapshot, setSnapshot] = useState<SystemJournalSnapshot | null>(null);
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(true);
  const [filter, setFilter] = useState('');
  const generation = useRef(0);
  const inFlight = useRef(false);

  useEffect(() => {
    const current = ++generation.current;
    inFlight.current = true;
    // Let a development StrictMode setup/cleanup replay invalidate the first
    // setup before it can issue remote I/O. This is not a timer or refresh loop.
    void Promise.resolve().then(() => {
      if (generation.current !== current) return null;
      return logsApi.list(host.id, sessionId);
    }).then((result) => {
      if (generation.current !== current || !result) return;
      if (result.hostId !== host.id || result.hostSessionId !== sessionId) {
        setFailed(true);
        return;
      }
      setSnapshot(result);
    }, () => {
      if (generation.current === current) setFailed(true);
    }).finally(() => {
      if (generation.current === current) {
        inFlight.current = false;
        setPending(false);
      }
    });
    return () => { generation.current += 1; };
  }, [host.id, sessionId]);

  function refresh() {
    if (inFlight.current) return;
    const current = generation.current;
    inFlight.current = true;
    setPending(true);
    setFailed(false);
    void logsApi.list(host.id, sessionId).then((result) => {
      if (generation.current !== current) return;
      if (result.hostId !== host.id || result.hostSessionId !== sessionId) {
        setFailed(true);
        return;
      }
      setSnapshot(result);
    }, () => {
      if (generation.current === current) setFailed(true);
    }).finally(() => {
      if (generation.current === current) {
        inFlight.current = false;
        setPending(false);
      }
    });
  }

  const rows = useMemo(() => {
    const term = filter.trim().toLocaleLowerCase();
    return snapshot?.entries.filter((entry) =>
      [entry.message ?? '', entry.unit ?? '', entry.identifier ?? '', entry.priority ? priorities[entry.priority] : '']
        .some((field) => field.toLocaleLowerCase().includes(term))) ?? [];
  }, [filter, snapshot]);

  return (
    <section aria-labelledby="logs-title">
      <header className="page-heading">
        <div><div className="eyebrow">READ-ONLY JOURNAL</div><h1 id="logs-title">Logs</h1><p>{host.displayName} · Connected</p></div>
        <Button onClick={refresh} disabled={pending}>Refresh</Button>
      </header>
      <p>Current boot · Up to 10 recent system journal entries accessible to this account. Refresh manually to update.</p>
      {pending && !snapshot && <Spinner label="Reading system journal…" />}
      {failed && <Notice>{snapshot ? 'Refresh failed — showing the last successful snapshot.' : unavailable}</Notice>}
      {snapshot && <>
        <p>Observed <time dateTime={snapshot.observedAt}>{snapshot.observedAt}</time> · {snapshot.entries.length} entries</p>
        <label htmlFor="logs-filter">Filter this snapshot</label>
        <input id="logs-filter" type="search" value={filter} onChange={(event) => setFilter(event.target.value)} />
        {snapshot.entries.length === 0 ? <p>No accessible system journal entries were returned.</p>
          : rows.length === 0 ? <p>No entries match this filter.</p>
          : <div className="files-table-wrap"><table className="files-table">
              <thead><tr><th scope="col">Time</th><th scope="col">Priority</th><th scope="col">Unit / Identifier</th><th scope="col">Message</th></tr></thead>
              <tbody>{rows.map((entry, index) => <tr key={index}>
                <td><time dateTime={entry.timestamp}>{entry.timestamp}</time></td>
                <td>{entry.priority ? priorities[entry.priority] : '—'}</td>
                <td style={{ overflowWrap: 'anywhere' }}>{entry.unit ?? '—'}<br />{entry.identifier ?? '—'}</td>
                <td style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', maxWidth: '48rem' }}>{entry.messageState === 'text'
                  ? entry.message : entry.messageState === 'missing' ? 'Message missing' : 'Message omitted'}</td>
              </tr>)}</tbody>
            </table></div>}
      </>}
    </section>
  );
}
