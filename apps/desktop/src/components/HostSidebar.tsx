import type { Host } from '@nexusops/protocol';
import { Button } from '@nexusops/ui';
import { Icon } from './Icon';
import { useHostSession } from '../api/queries';
import { ConnectionBadge } from './ConnectionBadge';

const futureSections = [
  ['services', 'Services'],
  ['containers', 'Containers'],
  ['network', 'Network'],
  ['security', 'Security'],
  ['logs', 'Logs'],
] as const;

function HostItem({
  host,
  selected,
  onSelect,
}: {
  host: Host;
  selected: boolean;
  onSelect: () => void;
}) {
  const session = useHostSession(host.id, 3_000);
  return (
    <button
      className={`host-item ${selected ? 'host-item--selected' : ''}`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="host-item-icon">
        <Icon name="server" size={16} />
      </span>
      <span className="host-item-content">
        <span className="host-item-name">{host.displayName}</span>
        <span className="host-item-address">{host.connection.hostname}</span>
      </span>
      <span className="host-item-state" title={session.data?.state ?? 'Loading status'}>
        {session.data ? (
          <ConnectionBadge state={session.data.state} />
        ) : (
          <span className="status-placeholder">·</span>
        )}
      </span>
    </button>
  );
}

export function HostSidebar({
  hosts,
  selectedId,
  onSelect,
  onAdd,
  activeSection,
  onSection,
}: {
  hosts: Host[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onAdd: () => void;
  activeSection: 'overview' | 'terminal' | 'files';
  onSection: (section: 'overview' | 'terminal' | 'files') => void;
}) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark" aria-hidden="true">
          N
        </span>
        <span>
          Nexus<span className="brand-light">Ops</span>
        </span>
        <span className="build-label">02</span>
      </div>
      <div className="workspace-label">LOCAL WORKSPACE</div>
      <label className="sr-only" htmlFor="host-selector">
        Select a host
      </label>
      <select
        id="host-selector"
        className="host-selector"
        value={selectedId ?? ''}
        onChange={(event) => onSelect(event.target.value || null)}
      >
        <option value="">All hosts</option>
        {hosts.map((host) => (
          <option key={host.id} value={host.id}>
            {host.displayName}
          </option>
        ))}
      </select>
      <nav className="navigation" aria-label="Host navigation">
        <button
          className={`nav-item ${activeSection === 'overview' ? 'nav-item--active' : ''}`}
          aria-current={activeSection === 'overview' ? 'page' : undefined}
          onClick={() => onSection('overview')}
        >
          <Icon name="overview" />
          Overview
        </button>
        <button
          className={`nav-item ${activeSection === 'terminal' ? 'nav-item--active' : ''}`}
          aria-current={activeSection === 'terminal' ? 'page' : undefined}
          disabled={!selectedId}
          onClick={() => onSection('terminal')}
        >
          <Icon name="terminal" />
          Terminal
        </button>
        <button
          className={`nav-item ${activeSection === 'files' ? 'nav-item--active' : ''}`}
          aria-current={activeSection === 'files' ? 'page' : undefined}
          disabled={!selectedId}
          onClick={() => onSection('files')}
        >
          <Icon name="files" />
          Files
        </button>
        {futureSections.map(([icon, label]) => (
          <button
            key={label}
            className="nav-item"
            disabled
            aria-label={`${label} (coming soon)`}
            title={`${label} is planned for a future release`}
          >
            <Icon name={icon} />
            <span>{label}</span>
            <span className="nav-future">Soon</span>
          </button>
        ))}
      </nav>
      <div className="sidebar-hosts-heading">
        <span>
          HOSTS <span className="count">{hosts.length}</span>
        </span>
        <Button variant="ghost" onClick={onAdd} aria-label="Add host">
          <Icon name="plus" size={16} />
        </Button>
      </div>
      <div className="sidebar-hosts" aria-label="Saved hosts">
        {hosts.length ? (
          hosts.map((host) => (
            <HostItem
              key={host.id}
              host={host}
              selected={selectedId === host.id}
              onSelect={() => onSelect(host.id)}
            />
          ))
        ) : (
          <p className="sidebar-empty">Your hosts will appear here.</p>
        )}
      </div>
      <div className="sidebar-footer">
        <Icon name="security" size={16} />
        <div>
          <strong>Direct. Agentless.</strong>
          <span>Secure SSH connections</span>
        </div>
        <span className="version">v0.1</span>
      </div>
    </aside>
  );
}
