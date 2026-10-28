import { useId, useRef, useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import type { ConnectionState, Host } from '@nexusops/protocol';
import { Button } from '@nexusops/ui';
import { Icon } from '../../components/Icon';
import { hostApi } from '../../api/client';
import { hostKeys } from '../../api/queries';
import { ConnectionBadge } from '../../components/ConnectionBadge';

type HostSort = 'name-asc' | 'name-desc' | 'hostname-asc' | 'hostname-desc';
type StatusFilter = 'all' | 'connected' | 'disconnected' | 'other' | 'unknown';

function compareText(left: string, right: string) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function HostRow({
  host,
  state,
  pending,
  onSelect,
}: {
  host: Host;
  state: ConnectionState | undefined;
  pending: boolean;
  onSelect: () => void;
}) {
  return (
    <tr>
      <td>
        <button type="button" className="host-table-name" onClick={onSelect}>
          <span className="host-row-icon">
            <Icon name="server" />
          </span>
          <span>
            {host.displayName}
            <span className="host-table-address">{host.connection.hostname}</span>
          </span>
        </button>
      </td>
      <td className="mono">{host.connection.username}</td>
      <td className="mono">{host.connection.port}</td>
      <td>{host.connection.authentication === 'privateKey' ? 'Private key' : 'Password'}</td>
      <td>
        {state ? <ConnectionBadge state={state} /> : pending ? 'Loading…' : 'Status unavailable'}
      </td>
      <td>
        <Button variant="ghost" onClick={onSelect} aria-label={`Open ${host.displayName}`}>
          <Icon name="arrow" size={17} />
        </Button>
      </td>
    </tr>
  );
}

export function HostList({
  hosts,
  onSelect,
  onAdd,
}: {
  hosts: Host[];
  onSelect: (id: string) => void;
  onAdd: () => void;
}) {
  const controlsId = useId();
  const searchInput = useRef<HTMLInputElement>(null);
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<HostSort>('name-asc');
  const [status, setStatus] = useState<StatusFilter>('all');
  // Observe every saved host independently of which rows are currently visible.
  // These are the existing session keys/options, also shared with HostSidebar.
  const sessions = useQueries({
    queries: hosts.map((host) => ({
      queryKey: hostKeys.session(host.id),
      queryFn: () => hostApi.session(host.id),
      refetchInterval: 3_000,
      retry: false,
    })),
  });
  const term = search.trim().toLowerCase();
  const rows = hosts
    .map((host, index) => {
      const query = sessions[index];
      // A failed refresh must not present a cached badge as current observation.
      const observation = query?.isSuccess ? query.data : undefined;
      const state = observation?.hostId === host.id ? observation.state : undefined;
      return { host, state, pending: query?.isPending ?? true };
    })
    .filter(({ host, state }) => {
      const matchesSearch = [
        host.displayName,
        host.connection.hostname,
        host.connection.username,
      ].some((value) => value.toLowerCase().includes(term));
      const matchesStatus =
        status === 'all' ||
        (status === 'unknown'
          ? state === undefined
          : status === 'other'
            ? state !== undefined && state !== 'connected' && state !== 'disconnected'
            : state === status);
      return matchesSearch && matchesStatus;
    })
    .sort((left, right) => {
      const field = (host: Host) =>
        (sort.startsWith('hostname') ? host.connection.hostname : host.displayName).toLowerCase();
      const direction = sort.endsWith('desc') ? -1 : 1;
      // ID ties always ascend, including when the primary field descends.
      return (
        direction * compareText(field(left.host), field(right.host)) ||
        compareText(left.host.id, right.host.id)
      );
    });
  const hasFilters = search !== '' || status !== 'all' || sort !== 'name-asc';

  function clearFilters() {
    setSearch('');
    setStatus('all');
    setSort('name-asc');
    searchInput.current?.focus();
  }

  return (
    <>
      <header className="page-heading">
        <div>
          <div className="eyebrow">LOCAL WORKSPACE</div>
          <h1>Your infrastructure</h1>
          <p className="page-description">
            A clear view of your servers. A direct connection to each one.
          </p>
        </div>
        <Button variant="primary" onClick={onAdd}>
          <Icon name="plus" size={16} />
          Add host
        </Button>
      </header>
      <div className="section-tabs">
        <span className="section-tab">
          All hosts<span className="tab-count">{hosts.length}</span>
        </span>
        <span className="section-tabs-note">SSH · agentless</span>
      </div>
      {hosts.length > 0 && (
        <>
          <section className="host-list-controls" aria-label="Host list controls">
            <label className="field host-list-search" htmlFor={`${controlsId}-search`}>
              Search hosts
              <input
                ref={searchInput}
                id={`${controlsId}-search`}
                type="search"
                placeholder="Name, hostname, or username"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </label>
            <label className="field" htmlFor={`${controlsId}-sort`}>
              Sort hosts
              <select
                id={`${controlsId}-sort`}
                value={sort}
                onChange={(event) => setSort(event.target.value as HostSort)}
              >
                <option value="name-asc">Display name — A to Z</option>
                <option value="name-desc">Display name — Z to A</option>
                <option value="hostname-asc">Hostname — A to Z</option>
                <option value="hostname-desc">Hostname — Z to A</option>
              </select>
            </label>
            <label className="field" htmlFor={`${controlsId}-status`}>
              Connection status
              <select
                id={`${controlsId}-status`}
                aria-describedby={`${controlsId}-status-help`}
                value={status}
                onChange={(event) => setStatus(event.target.value as StatusFilter)}
              >
                <option value="all">All hosts</option>
                <option value="connected">Connected</option>
                <option value="disconnected">Disconnected</option>
                <option value="other">Other known states</option>
                <option value="unknown">Unknown / loading</option>
              </select>
            </label>
            <span id={`${controlsId}-status-help`} className="sr-only">
              Other known states include connecting, awaiting identity verification, disconnecting,
              and connection failures. Unknown includes unavailable connection observations.
            </span>
            {hasFilters && (
              <Button type="button" onClick={clearFilters}>
                Clear filters
              </Button>
            )}
          </section>
          <p className="host-list-summary" role="status" aria-live="polite" aria-atomic="true">
            Showing {rows.length} of {hosts.length} {hosts.length === 1 ? 'host' : 'hosts'}
          </p>
        </>
      )}
      {hosts.length ? (
        rows.length ? (
          <section className="panel host-table-panel" aria-label="Host configurations">
            <table className="host-table">
              <thead>
                <tr>
                  <th scope="col">HOST</th>
                  <th scope="col">USER</th>
                  <th scope="col">PORT</th>
                  <th scope="col">AUTHENTICATION</th>
                  <th scope="col">STATUS</th>
                  <th scope="col">
                    <span className="sr-only">Open</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.map(({ host, state, pending }) => (
                  <HostRow
                    key={host.id}
                    host={host}
                    state={state}
                    pending={pending}
                    onSelect={() => onSelect(host.id)}
                  />
                ))}
              </tbody>
            </table>
          </section>
        ) : (
          <section className="panel host-list-empty" aria-label="No matching hosts">
            <h2>No matching hosts</h2>
            <p>Try a different search or connection status, or clear your filters.</p>
          </section>
        )
      ) : (
        <section className="first-host">
          <div className="first-host-illustration" aria-hidden="true">
            <span className="host-outline host-outline--back" />
            <span className="host-outline">
              <span className="server-line">
                <i />
                <i />
                <b />
              </span>
              <span className="server-line">
                <i />
                <i />
                <b />
              </span>
            </span>
            <span className="connection-stem" />
            <span className="secure-marker">
              <Icon name="security" size={18} />
            </span>
          </div>
          <div className="eyebrow">START WITH A CONNECTION</div>
          <h2>Your next server starts here.</h2>
          <p>
            No saved hosts yet. Add a Linux host to inspect its system information over a secure SSH
            connection.
          </p>
          <Button variant="primary" onClick={onAdd}>
            <Icon name="plus" size={16} />
            Add your first host
          </Button>
          <div className="empty-benefits">
            <span>
              <Icon name="security" size={16} />
              Your credentials stay local
            </span>
            <span>
              <Icon name="server" size={16} />
              Nothing to install on your server
            </span>
          </div>
        </section>
      )}
      <div className="workspace-note">
        <Icon name="shield" size={18} />
        <p>
          <strong>Built for deliberate operations.</strong> This release supports secure connections
          and read-only host discovery. Additional tools will appear as they become available.
        </p>
      </div>
    </>
  );
}
