import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  Host,
  ListeningPortSnapshot,
  NetworkRouteSnapshot,
  NetworkSnapshot,
} from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { networkApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

type Observation<T> = { snapshot: T | null; error: boolean; pending: boolean };
const initial = { snapshot: null, error: false, pending: true };

export function NetworkWorkspace({ host }: { host: Host }) {
  const session = useHostSession(host.id).data;
  const sessionId = session?.state === 'connected' ? session.hostSessionId : null;
  if (sessionId)
    return <ConnectedNetwork key={`${host.id}:${sessionId}`} host={host} sessionId={sessionId} />;
  return (
    <section aria-labelledby="network-title">
      <header className="page-heading">
        <div>
          <div className="eyebrow">READ-ONLY INVENTORY</div>
          <h1 id="network-title">Network</h1>
          <p>{host.displayName} · Disconnected</p>
        </div>
        <Button disabled>Refresh</Button>
      </header>
      <p>Connect this host to inspect network interfaces.</p>
    </section>
  );
}

// Remount on ownership changes: topology and errors never survive an old host/session.
function ConnectedNetwork({ host, sessionId }: { host: Host; sessionId: string }) {
  const [interfaces, setInterfaces] = useState<Observation<NetworkSnapshot>>(initial);
  const [routes, setRoutes] = useState<Observation<NetworkRouteSnapshot>>(initial);
  const [ports, setPorts] = useState<Observation<ListeningPortSnapshot>>(initial);
  const [portSearch, setPortSearch] = useState('');
  const [portProtocol, setPortProtocol] = useState('all');
  const [filter, setFilter] = useState('');
  const [routeFilter, setRouteFilter] = useState('');
  const [family, setFamily] = useState('all');
  const generation = useRef(0);
  const inFlight = useRef(false);

  const load = useCallback(
    async (current: number) => {
      if (inFlight.current || generation.current !== current) return;
      inFlight.current = true;
      setInterfaces((previous) => ({ ...previous, error: false, pending: true }));
      setRoutes((previous) => ({ ...previous, error: false, pending: true }));
      setPorts((previous) => ({ ...previous, error: false, pending: true }));
      try {
        const result = await networkApi.list(host.id, sessionId);
        if (generation.current !== current) return;
        if (result.hostId !== host.id || result.hostSessionId !== sessionId)
          throw new Error('Snapshot ownership mismatch');
        setInterfaces({ snapshot: result, error: false, pending: false });
      } catch {
        if (generation.current !== current) return;
        setInterfaces((previous) => ({ ...previous, error: true, pending: false }));
      }
      // Address and route operations share the backend Network gate. Always wait
      // for the first operation to settle, including failure, before reading routes.
      if (generation.current !== current) return;
      try {
        const result = await networkApi.routes(host.id, sessionId);
        if (generation.current !== current) return;
        if (result.hostId !== host.id || result.hostSessionId !== sessionId)
          throw new Error('Snapshot ownership mismatch');
        setRoutes({ snapshot: result, error: false, pending: false });
      } catch {
        if (generation.current !== current) return;
        setRoutes((previous) => ({ ...previous, error: true, pending: false }));
      }
      if (generation.current !== current) return;
      try {
        const result = await networkApi.listeningPorts(host.id, sessionId);
        if (generation.current !== current) return;
        if (result.hostId !== host.id || result.hostSessionId !== sessionId)
          throw new Error('Snapshot ownership mismatch');
        setPorts({ snapshot: result, error: false, pending: false });
      } catch {
        if (generation.current !== current) return;
        setPorts((previous) => ({ ...previous, error: true, pending: false }));
      } finally {
        if (generation.current === current) inFlight.current = false;
      }
    },
    [host.id, sessionId],
  );

  useEffect(() => {
    const current = ++generation.current;
    // Skip the discarded StrictMode setup before starting any IPC request.
    void Promise.resolve().then(() => {
      if (generation.current === current) void load(current);
    });
    return () => {
      generation.current += 1;
    };
  }, [load]);

  const portSnapshot = ports.snapshot;
  const portRows = useMemo(() => {
    const term = portSearch.trim().toLowerCase();
    return (
      portSnapshot?.entries.filter(
        (entry) =>
          (portProtocol === 'all' || entry.protocol === portProtocol) &&
          [entry.localAddress, String(entry.port), entry.protocol, entry.family, entry.state].some(
            (field) => field.toLowerCase().includes(term),
          ),
      ) ?? []
    );
  }, [portProtocol, portSearch, portSnapshot]);
  const snapshot = interfaces.snapshot;
  const routeSnapshot = routes.snapshot;
  const rows = useMemo(() => {
    const term = filter.trim().toLowerCase();
    return (
      snapshot?.entries.filter((entry) =>
        [
          entry.name,
          entry.operState ?? '',
          ...entry.addresses.map((address) => `${address.address}/${address.prefixLength}`),
        ].some((field) => field.toLowerCase().includes(term)),
      ) ?? []
    );
  }, [filter, snapshot]);
  const routeRows = useMemo(() => {
    const term = routeFilter.trim().toLowerCase();
    return (
      routeSnapshot?.entries.filter(
        (entry) =>
          (family === 'all' || entry.family === family) &&
          [entry.destination, entry.gateway ?? '', entry.interface].some((field) =>
            field.toLowerCase().includes(term),
          ),
      ) ?? []
    );
  }, [family, routeFilter, routeSnapshot]);
  const addressCount =
    snapshot?.entries.reduce((total, entry) => total + entry.addresses.length, 0) ?? 0;
  const defaultCount =
    routeSnapshot?.entries.filter((entry) => entry.destination === 'default').length ?? 0;
  return (
    <section aria-labelledby="network-title">
      <header className="page-heading">
        <div>
          <div className="eyebrow">READ-ONLY INVENTORY</div>
          <h1 id="network-title">Network</h1>
          <p>{host.displayName} · Connected</p>
        </div>
        <Button
          onClick={() => void load(generation.current)}
          disabled={interfaces.pending || routes.pending || ports.pending}
        >
          Refresh
        </Button>
      </header>
      <section aria-labelledby="network-interfaces-title">
        <h2 id="network-interfaces-title">Interfaces &amp; Addresses</h2>
        {interfaces.pending && !snapshot && <Spinner label="Reading network interfaces…" />}
        {interfaces.error && (
          <Notice>
            {snapshot
              ? 'Refresh failed — showing the last successful snapshot.'
              : 'Network inventory is unavailable on this host.'}
          </Notice>
        )}
        {snapshot && (
          <>
            <p>
              Observed <time dateTime={snapshot.observedAt}>{snapshot.observedAt}</time> ·{' '}
              {snapshot.entries.length} interfaces · {addressCount} addresses
            </p>
            <label htmlFor="network-filter">Filter interfaces and addresses</label>
            <input
              id="network-filter"
              type="search"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
            />
            {snapshot.entries.length === 0 ? (
              <p>No network interfaces were returned.</p>
            ) : rows.length === 0 ? (
              <p>No interfaces match this filter.</p>
            ) : (
              <div className="files-table-wrap">
                <table className="files-table">
                  <thead>
                    <tr>
                      <th scope="col">Interface</th>
                      <th scope="col">State</th>
                      <th scope="col">MTU</th>
                      <th scope="col">Addresses</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((entry) => (
                      <tr key={entry.ifindex}>
                        <td>{entry.name}</td>
                        <td>{entry.operState ?? '—'}</td>
                        <td>{entry.mtu ?? '—'}</td>
                        <td>
                          {entry.addresses.length === 0
                            ? '—'
                            : entry.addresses.map((address) => (
                                <div
                                  key={`${address.family}:${address.address}/${address.prefixLength}`}
                                >
                                  {address.family === 'ipv4' ? 'IPv4' : 'IPv6'} {address.address}/
                                  {address.prefixLength}
                                </div>
                              ))}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}
      </section>
      <section className="network-routes" aria-labelledby="network-routes-title">
        <h2 id="network-routes-title">Routes &amp; Gateway</h2>
        <p>
          Main routing table only. This does not include other tables, policy rules or the route
          selected for a particular packet.
        </p>
        {routes.pending && !routeSnapshot && <Spinner label="Reading main-table routes…" />}
        {routes.error && (
          <Notice>
            {routeSnapshot
              ? 'Route refresh failed — showing the last successful same-session route snapshot.'
              : 'Route inventory is unavailable on this host.'}
          </Notice>
        )}
        {routeSnapshot && (
          <>
            <p>
              Routes observed{' '}
              <time dateTime={routeSnapshot.observedAt}>{routeSnapshot.observedAt}</time> ·{' '}
              {routeSnapshot.entries.length} routes
            </p>
            {defaultCount === 0 ? (
              <p>
                No default gateway is observed in the main table. This does not establish whether
                external connectivity is available.
              </p>
            ) : (
              <p>
                {defaultCount} default route(s) observed; no uniquely preferred gateway is inferred.
              </p>
            )}
            {(['ipv4', 'ipv6'] as const).map(
              (value) =>
                !routeSnapshot.entries.some((entry) => entry.family === value) && (
                  <p key={value}>
                    No {value === 'ipv4' ? 'IPv4' : 'IPv6'} routes were returned from the main
                    table.
                  </p>
                ),
            )}
            <div className="network-route-controls">
              <label>
                Search routes
                <input
                  type="search"
                  value={routeFilter}
                  onChange={(event) => setRouteFilter(event.target.value)}
                />
              </label>
              <label>
                Route address family
                <select value={family} onChange={(event) => setFamily(event.target.value)}>
                  <option value="all">IPv4 and IPv6</option>
                  <option value="ipv4">IPv4</option>
                  <option value="ipv6">IPv6</option>
                </select>
              </label>
            </div>
            <p role="status">
              Showing {routeRows.length} of {routeSnapshot.entries.length} routes
            </p>
            {routeRows.length === 0 ? (
              <p>
                {routeSnapshot.entries.length === 0
                  ? 'No main-table routes were returned.'
                  : 'No routes match these filters.'}
              </p>
            ) : (
              <div className="files-table-wrap">
                <table className="files-table">
                  <caption className="sr-only">Main routing table routes</caption>
                  <thead>
                    <tr>
                      <th scope="col">Destination</th>
                      <th scope="col">Gateway</th>
                      <th scope="col">Interface</th>
                      <th scope="col">Metric</th>
                      <th scope="col">Family</th>
                      <th scope="col">Preference</th>
                    </tr>
                  </thead>
                  <tbody>
                    {routeRows.map((entry) => (
                      <tr
                        key={JSON.stringify([
                          entry.family,
                          entry.destination,
                          entry.gateway,
                          entry.interface,
                          entry.metric,
                        ])}
                      >
                        <td>{entry.destination}</td>
                        <td>
                          {entry.gateway ?? 'Direct / on-link'}
                          {entry.onLink && entry.gateway ? ' (on-link)' : ''}
                        </td>
                        <td>
                          {entry.interface}
                          {entry.linkDown ? ' (link down)' : ''}
                        </td>
                        <td>{entry.metric ?? 'Not reported'}</td>
                        <td>{entry.family === 'ipv4' ? 'IPv4' : 'IPv6'}</td>
                        <td>{entry.preference ?? 'Not reported'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}
      </section>
      <section className="network-routes" aria-labelledby="network-ports-title">
        <h2 id="network-ports-title">Listening Ports</h2>
        <p>
          A local listening socket does not prove external reachability or firewall exposure. UDP
          bound sockets do not prove that a reachable service is accepting traffic.
        </p>
        {ports.pending && !portSnapshot && <Spinner label="Reading listening ports…" />}
        {ports.error && (
          <Notice>
            {portSnapshot
              ? 'Listening-port refresh failed — showing the last successful same-session socket snapshot.'
              : 'Listening-port inventory is unavailable on this host.'}
          </Notice>
        )}
        {portSnapshot && (
          <>
            <p>
              Listening ports observed{' '}
              <time dateTime={portSnapshot.observedAt}>{portSnapshot.observedAt}</time>
            </p>
            <p className="network-socket-summary">
              TCP listening:{' '}
              {portSnapshot.entries.filter((entry) => entry.protocol === 'tcp').length} · UDP bound
              sockets: {portSnapshot.entries.filter((entry) => entry.protocol === 'udp').length}
            </p>
            <div className="network-route-controls">
              <label>
                Search listening ports
                <input
                  type="search"
                  value={portSearch}
                  onChange={(event) => setPortSearch(event.target.value)}
                />
              </label>
              <label>
                Socket protocol
                <select
                  value={portProtocol}
                  onChange={(event) => setPortProtocol(event.target.value)}
                >
                  <option value="all">TCP and UDP</option>
                  <option value="tcp">TCP</option>
                  <option value="udp">UDP</option>
                </select>
              </label>
            </div>
            <p role="status">
              Showing {portRows.length} of {portSnapshot.entries.length} sockets
            </p>
            {portRows.length === 0 ? (
              <p>
                {portSnapshot.entries.length === 0
                  ? 'No supported listening or bound sockets were returned.'
                  : 'No sockets match these filters.'}
              </p>
            ) : (
              <div className="files-table-wrap">
                <table className="files-table">
                  <caption className="sr-only">Listening and bound sockets</caption>
                  <thead>
                    <tr>
                      <th scope="col">Protocol</th>
                      <th scope="col">Local bind address</th>
                      <th scope="col">Port</th>
                      <th scope="col">Address family</th>
                      <th scope="col">Reported state</th>
                    </tr>
                  </thead>
                  <tbody>
                    {portRows.map((entry) => (
                      <tr
                        key={JSON.stringify([
                          entry.protocol,
                          entry.family,
                          entry.localAddress,
                          entry.port,
                          entry.state,
                        ])}
                      >
                        <td>{entry.protocol.toUpperCase()}</td>
                        <td>{entry.localAddress}</td>
                        <td>{entry.port}</td>
                        <td>
                          {entry.family === 'ipv4'
                            ? 'IPv4'
                            : entry.family === 'ipv6'
                              ? 'IPv6'
                              : 'Unknown'}
                        </td>
                        <td>{entry.state.toUpperCase()}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}
      </section>
    </section>
  );
}
