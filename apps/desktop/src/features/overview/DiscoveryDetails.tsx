import type { DiscoverySnapshot, HostSession } from '@nexusops/protocol';
import { Badge, Notice } from '@nexusops/ui';
import { bytes, observedTime, percentage, uptime } from './format';
import { Icon } from '../../components/Icon';

function ResourceMetric({
  title,
  used,
  total,
}: {
  title: string;
  used: number | null;
  total: number | null;
}) {
  const percent = percentage(used, total);
  return (
    <div className="metric">
      <span className="metric-label">{title}</span>
      <div className="metric-value">
        {used === null ? 'Unavailable' : bytes(used)}
        <span className="metric-total">{total === null ? '' : ` / ${bytes(total)}`}</span>
      </div>
      {percent === null ? (
        <div className="metric-unavailable">Usage data unavailable</div>
      ) : (
        <>
          <div
            className="resource-track"
            role="meter"
            aria-label={`${title} usage`}
            aria-valuenow={Math.round(percent)}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <span style={{ width: `${percent}%` }} />
          </div>
          <div className="metric-caption">{Math.round(percent)}% utilized</div>
        </>
      )}
    </div>
  );
}

export function DiscoveryDetails({
  discovery,
  session,
}: {
  discovery: DiscoverySnapshot;
  session: HostSession;
}) {
  const details = [
    ['Hostname', discovery.hostname],
    ['Operating system', discovery.os],
    ['OS version', discovery.osVersion],
    ['Kernel', discovery.kernel],
    ['Architecture', discovery.architecture],
  ];
  return (
    <>
      <section className="metrics-grid" aria-label="Host resources">
        <div className="metric">
          <span className="metric-label">UPTIME</span>
          <div className="metric-value">{uptime(discovery.uptimeSeconds)}</div>
          <div className="metric-caption">Since last system boot</div>
        </div>
        <div className="metric">
          <span className="metric-label">SYSTEM LOAD</span>
          <div className="metric-value">
            {discovery.loadOne === null ? 'Unavailable' : discovery.loadOne.toFixed(2)}
          </div>
          <div className="metric-caption">1-minute load average</div>
        </div>
        <ResourceMetric
          title="MEMORY"
          used={discovery.memoryUsedBytes}
          total={discovery.memoryTotalBytes}
        />
        <ResourceMetric
          title="ROOT FILESYSTEM"
          used={discovery.rootUsedBytes}
          total={discovery.rootTotalBytes}
        />
      </section>
      <div className="overview-columns">
        <section className="panel">
          <div className="panel-heading">
            <h2>System information</h2>
            <span className="panel-caption">Linux host</span>
          </div>
          <dl className="detail-list">
            {details.map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{value || <span className="unavailable">Unavailable</span>}</dd>
              </div>
            ))}
          </dl>
        </section>
        <section className="panel">
          <div className="panel-heading">
            <h2>Connection security</h2>
            <Badge tone="success">Verified</Badge>
          </div>
          <div className="security-summary">
            <span className="security-icon">
              <Icon name="security" size={22} />
            </span>
            <div>
              <strong>Trusted host identity</strong>
              <p>The server key matches your locally stored fingerprint.</p>
            </div>
          </div>
          <dl className="detail-list compact">
            <div>
              <dt>Key algorithm</dt>
              <dd>{session.identity?.fingerprint.algorithm ?? 'Unavailable'}</dd>
            </div>
            <div className="detail-stacked">
              <dt>SHA-256 fingerprint</dt>
              <dd className="fingerprint">
                {session.identity?.fingerprint.sha256 ?? 'Unavailable'}
              </dd>
            </div>
          </dl>
        </section>
      </div>
      <section className="panel capability-panel">
        <div className="panel-heading">
          <h2>Available capabilities</h2>
          <span className="panel-caption">Discovery registry</span>
        </div>
        <div className="capabilities">
          {session.capabilities.length ? (
            session.capabilities.map((capability) => (
              <span
                key={capability.id}
                className={`capability ${capability.available ? '' : 'capability--unavailable'}`}
              >
                <Icon name="services" size={14} />
                {capability.id}
                <span>{capability.available ? 'Available' : 'Unavailable'}</span>
              </span>
            ))
          ) : (
            <span className="muted">No capabilities reported for this host.</span>
          )}
        </div>
      </section>
      {discovery.warnings.length > 0 && (
        <Notice tone="warning">
          <strong>Some discovery information is unavailable.</strong>
          <ul className="warning-list">
            {discovery.warnings.map((warning, index) => (
              <li key={`${warning.code}-${index}`}>{warning.message}</li>
            ))}
          </ul>
        </Notice>
      )}
      <div className="discovery-footer">
        <span>
          <Icon name="security" size={14} />
          Read-only discovery · no remote agent
        </span>
        <span>Observed {observedTime(discovery.observedAt)}</span>
      </div>
    </>
  );
}
