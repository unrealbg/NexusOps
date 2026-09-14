import type { Host } from '@nexusops/protocol';
import { Button } from '@nexusops/ui';
import { Icon } from '../../components/Icon';
import { useHostSession } from '../../api/queries';
import { ConnectionBadge } from '../../components/ConnectionBadge';

function HostRow({ host, onSelect }: { host: Host; onSelect: () => void }) {
  const session = useHostSession(host.id, 3_000);
  return (
    <tr>
      <td>
        <button className="host-table-name" onClick={onSelect}>
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
      <td>{session.data ? <ConnectionBadge state={session.data.state} /> : 'Loading…'}</td>
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
      {hosts.length ? (
        <section className="panel host-table-panel" aria-label="Host configurations">
          <table className="host-table">
            <thead>
              <tr>
                <th>HOST</th>
                <th>USER</th>
                <th>PORT</th>
                <th>AUTHENTICATION</th>
                <th>STATUS</th>
                <th>
                  <span className="sr-only">Open</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {hosts.map((host) => (
                <HostRow key={host.id} host={host} onSelect={() => onSelect(host.id)} />
              ))}
            </tbody>
          </table>
        </section>
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
          <p>Add a Linux host to inspect its system information over a secure SSH connection.</p>
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
