import { useEffect, useMemo, useRef, useState } from 'react';
import type { Host, ServiceSnapshot } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { servicesApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

export function ServicesWorkspace({ host }: { host: Host }) {
  const sessionQuery = useHostSession(host.id);
  const session = sessionQuery.data;
  const connected = session?.state === 'connected';
  const sessionId = connected ? session.hostSessionId : null;
  const owner = `${host.id}:${sessionId ?? ''}`;
  const [snapshot, setSnapshot] = useState<ServiceSnapshot | null>(null);
  const [error, setError] = useState<{ owner: string; failedRefresh: boolean } | null>(null);
  const [refreshingOwner, setRefreshingOwner] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const generation = useRef(0);
  const inFlight = useRef(false);

  useEffect(() => {
    const current = ++generation.current;
    inFlight.current = false;
    if (!connected || !sessionId) return () => { generation.current += 1; };
    inFlight.current = true;
    void servicesApi.list(host.id, sessionId).then(
      (result) => {
        if (generation.current !== current) return;
        if (result.hostId !== host.id || result.hostSessionId !== sessionId) {
          setError({ owner, failedRefresh: false });
          return;
        }
        setSnapshot(result);
      },
      () => {
        if (generation.current === current) setError({ owner, failedRefresh: false });
      },
    ).finally(() => {
      if (generation.current === current) {
        inFlight.current = false;
      }
    });
    return () => { generation.current += 1; };
  }, [connected, host.id, owner, sessionId]);

  const validSnapshot = connected && snapshot?.hostId === host.id && snapshot.hostSessionId === sessionId
    ? snapshot : null;
  const visibleError = error?.owner === owner ? error : null;
  const pending = refreshingOwner === owner || (connected && !validSnapshot && !visibleError);
  const rows = useMemo(() => {
    if (!validSnapshot) return [];
    const term = filter.trim().toLocaleLowerCase();
    if (!term) return validSnapshot.entries;
    return validSnapshot.entries.filter((entry) =>
      [entry.unit, entry.activeState, entry.subState, entry.description]
        .some((field) => field.toLocaleLowerCase().includes(term)),
    );
  }, [filter, validSnapshot]);

  function refresh() {
    if (!connected || !sessionId || inFlight.current) return;
    const current = generation.current;
    const hadSnapshot = validSnapshot !== null;
    inFlight.current = true;
    setRefreshingOwner(owner);
    setError(null);
    void servicesApi.list(host.id, sessionId).then(
      (result) => {
        if (generation.current !== current) return;
        if (result.hostId !== host.id || result.hostSessionId !== sessionId) {
          setError({ owner, failedRefresh: hadSnapshot });
          return;
        }
        setSnapshot(result);
      },
      () => {
        if (generation.current === current) setError({ owner, failedRefresh: hadSnapshot });
      },
    ).finally(() => {
      if (generation.current === current) {
        inFlight.current = false;
        setRefreshingOwner(null);
      }
    });
  }

  return (
    <section aria-labelledby="services-title">
      <header className="page-heading">
        <div><div className="eyebrow">READ-ONLY INVENTORY</div><h1 id="services-title">Services</h1><p>{host.displayName} · {connected ? 'Connected' : 'Disconnected'}</p></div>
        <Button onClick={refresh} disabled={!connected || !sessionId || pending}>Refresh</Button>
      </header>
      {!connected ? <p>Connect this host to inspect system services.</p> : (
        <>
          {pending && !validSnapshot && <Spinner label="Reading system services…" />}
          {visibleError && (
            <Notice>{visibleError.failedRefresh && validSnapshot
              ? 'Refresh failed — showing the last successful snapshot.'
              : 'Service inventory is unavailable on this host.'}</Notice>
          )}
          {validSnapshot && (
            <>
              <p>Observed <time dateTime={validSnapshot.observedAt}>{validSnapshot.observedAt}</time> · {rows.length} of {validSnapshot.entries.length} services</p>
              <label htmlFor="service-filter">Filter services</label>
              <input id="service-filter" type="search" value={filter} onChange={(event) => setFilter(event.target.value)} />
              {validSnapshot.entries.length === 0 ? <p>No loaded system services were returned.</p>
                : rows.length === 0 ? <p>No services match this filter.</p>
                : <div className="files-table-wrap"><table className="files-table"><thead><tr><th scope="col">Service</th><th scope="col">Load</th><th scope="col">Active</th><th scope="col">Sub</th><th scope="col">Description</th></tr></thead><tbody>
                    {rows.map((entry) => <tr key={entry.unit}><td>{entry.unit}</td><td>{entry.loadState}</td><td>{entry.activeState}</td><td>{entry.subState}</td><td>{entry.description}</td></tr>)}
                  </tbody></table></div>}
            </>
          )}
        </>
      )}
    </section>
  );
}
