import { useEffect, useMemo, useRef, useState } from 'react';
import type { Host, NetworkSnapshot } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { networkApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

export function NetworkWorkspace({ host }: { host: Host }) {
  const session = useHostSession(host.id).data;
  const connected = session?.state === 'connected';
  const sessionId = connected ? session.hostSessionId : null;
  const owner = `${host.id}:${sessionId ?? ''}`;
  const [snapshot, setSnapshot] = useState<NetworkSnapshot | null>(null);
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
    void networkApi.list(host.id, sessionId).then(
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
      if (generation.current === current) inFlight.current = false;
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
      [entry.name, entry.operState ?? '', ...entry.addresses.map((address) =>
        `${address.address}/${address.prefixLength}`)]
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
    void networkApi.list(host.id, sessionId).then(
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

  const addressCount = validSnapshot?.entries.reduce((total, entry) => total + entry.addresses.length, 0) ?? 0;
  return (
    <section aria-labelledby="network-title">
      <header className="page-heading">
        <div><div className="eyebrow">READ-ONLY INVENTORY</div><h1 id="network-title">Network</h1><p>{host.displayName} · {connected ? 'Connected' : 'Disconnected'}</p></div>
        <Button onClick={refresh} disabled={!connected || !sessionId || pending}>Refresh</Button>
      </header>
      {!connected ? <p>Connect this host to inspect network interfaces.</p> : (
        <>
          {pending && !validSnapshot && <Spinner label="Reading network interfaces…" />}
          {visibleError && (
            <Notice>{visibleError.failedRefresh && validSnapshot
              ? 'Refresh failed — showing the last successful snapshot.'
              : 'Network inventory is unavailable on this host.'}</Notice>
          )}
          {validSnapshot && (
            <>
              <p>Observed <time dateTime={validSnapshot.observedAt}>{validSnapshot.observedAt}</time> · {validSnapshot.entries.length} interfaces · {addressCount} addresses</p>
              <label htmlFor="network-filter">Filter interfaces and addresses</label>
              <input id="network-filter" type="search" value={filter} onChange={(event) => setFilter(event.target.value)} />
              {validSnapshot.entries.length === 0 ? <p>No network interfaces were returned.</p>
                : rows.length === 0 ? <p>No interfaces match this filter.</p>
                : <div className="files-table-wrap"><table className="files-table"><thead><tr><th scope="col">Interface</th><th scope="col">State</th><th scope="col">MTU</th><th scope="col">Addresses</th></tr></thead><tbody>
                    {rows.map((entry) => <tr key={entry.ifindex}><td>{entry.name}</td><td>{entry.operState ?? '—'}</td><td>{entry.mtu ?? '—'}</td><td>{entry.addresses.length === 0 ? '—' : entry.addresses.map((address) => <div key={`${address.family}:${address.address}/${address.prefixLength}`}>{address.family === 'ipv4' ? 'IPv4' : 'IPv6'} {address.address}/{address.prefixLength}</div>)}</td></tr>)}
                  </tbody></table></div>}
            </>
          )}
        </>
      )}
    </section>
  );
}
