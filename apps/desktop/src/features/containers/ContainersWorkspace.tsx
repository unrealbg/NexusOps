import { useEffect, useMemo, useRef, useState } from 'react';
import type { ContainerSnapshot, ContainerState, Host } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { containersApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

const states: Record<ContainerState, string> = {
  created: 'Created', restarting: 'Restarting', running: 'Running', removing: 'Removing',
  paused: 'Paused', exited: 'Exited', dead: 'Dead',
};
const unavailable = 'Docker container inventory is unavailable for this connection.';

export function ContainersWorkspace({ host }: { host: Host }) {
  const sessionQuery = useHostSession(host.id);
  const session = !sessionQuery.isError && sessionQuery.data?.hostId === host.id ? sessionQuery.data : null;
  const sessionId = session?.state === 'connected' ? session.hostSessionId : null;
  // An errored, missing or wrong-host query cannot retain prior container authority.
  return sessionId ? <ConnectedContainers key={`${host.id}:${sessionId}`} host={host} sessionId={sessionId} /> : (
    <section aria-labelledby="containers-title">
      <header className="page-heading">
        <div><div className="eyebrow">READ-ONLY DOCKER INVENTORY</div><h1 id="containers-title">Containers</h1><p>{host.displayName}</p></div>
        <Button disabled>Refresh</Button>
      </header>
      <p>Docker · local system daemon</p>
      <p>{sessionQuery.isPending && !sessionQuery.isError && !sessionQuery.data
        ? 'Reading connection state…'
        : !session || session.state === 'connected'
          ? 'Current SSH session state is unavailable.'
          : 'Connect this host to inspect local Docker containers.'}</p>
    </section>
  );
}

function ConnectedContainers({ host, sessionId }: { host: Host; sessionId: string }) {
  const [snapshot, setSnapshot] = useState<ContainerSnapshot | null>(null);
  const [failed, setFailed] = useState(false);
  const [pending, setPending] = useState(true);
  const [filter, setFilter] = useState('');
  const generation = useRef(0);
  const inFlight = useRef(false);

  useEffect(() => {
    const current = ++generation.current;
    inFlight.current = true;
    // Defer initial I/O so StrictMode setup/cleanup replay cannot duplicate it.
    void Promise.resolve().then(() => {
      if (generation.current !== current) return null;
      return containersApi.list(host.id, sessionId);
    }).then((result) => {
      if (generation.current !== current || !result) return;
      if (result.hostId !== host.id || result.hostSessionId !== sessionId || result.provider !== 'dockerSystem') {
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
    void containersApi.list(host.id, sessionId).then((result) => {
      if (generation.current !== current) return;
      if (result.hostId !== host.id || result.hostSessionId !== sessionId || result.provider !== 'dockerSystem') {
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
      [entry.name, entry.id, entry.image, states[entry.state], entry.status, entry.ports, entry.networks]
        .some((field) => field.toLocaleLowerCase().includes(term))) ?? [];
  }, [filter, snapshot]);

  return (
    <section aria-labelledby="containers-title">
      <header className="page-heading">
        <div><div className="eyebrow">READ-ONLY DOCKER INVENTORY</div><h1 id="containers-title">Containers</h1><p>{host.displayName} · Connected</p></div>
        <Button onClick={refresh} disabled={pending}>Refresh</Button>
      </header>
      <p>Docker · local system daemon · Up to 64 most recently created containers in all states. Refresh manually to update.</p>
      {pending && !snapshot && <Spinner label="Reading local Docker inventory…" />}
      {failed && <Notice>{snapshot ? 'Refresh failed — showing the last successful snapshot.' : unavailable}</Notice>}
      {snapshot && <>
        <p>Observed <time dateTime={snapshot.observedAt}>{snapshot.observedAt}</time> · {snapshot.entries.length} containers</p>
        <label htmlFor="containers-filter">Filter this snapshot</label>
        <input id="containers-filter" type="search" value={filter} onChange={(event) => setFilter(event.target.value)} />
        {snapshot.entries.length === 0 ? <p>No containers were returned by the local Docker daemon.</p>
          : rows.length === 0 ? <p>No containers match this filter.</p>
          : <div className="files-table-wrap"><table className="files-table">
              <thead><tr><th scope="col">Name</th><th scope="col">ID</th><th scope="col">State</th><th scope="col">Image</th><th scope="col">Status</th><th scope="col">Ports</th><th scope="col">Networks</th></tr></thead>
              <tbody>{rows.map((entry, index) => <tr key={`${entry.id}-${index}`}>
                <td style={{ overflowWrap: 'anywhere' }}>{entry.name}</td>
                <td>{entry.id.slice(0, 12)}</td>
                <td>{states[entry.state]}</td>
                <td style={{ overflowWrap: 'anywhere' }}>{entry.image}</td>
                <td style={{ overflowWrap: 'anywhere' }}>{entry.status}</td>
                <td style={{ overflowWrap: 'anywhere' }}>{entry.ports || '—'}</td>
                <td style={{ overflowWrap: 'anywhere' }}>{entry.networks || '—'}</td>
              </tr>)}</tbody>
            </table></div>}
      </>}
    </section>
  );
}
