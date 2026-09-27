import { useCallback, useEffect, useRef, useState } from 'react';
import type { Host, HostSession, SshEndpointTrust } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { securityApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

function sessionStatus(session: HostSession, trust: SshEndpointTrust | null): string {
  switch (session.state) {
    case 'connecting': return 'Host-key verification in progress';
    case 'awaitingTrust': return 'Awaiting first-contact trust';
    case 'connected': {
      const identity = session.identity;
      const pin = trust?.endpointPin;
      return session.hostSessionId && identity && pin
        && identity.hostname === trust.hostname
        && identity.fingerprint.algorithm === pin.algorithm
        && identity.fingerprint.sha256 === pin.sha256
        ? 'Verified against endpoint pin at session establishment'
        : 'Session verification unavailable: endpoint trust metadata is missing or inconsistent.';
    }
    default: return 'No active verified session';
  }
}

const connectionLabels = {
  disconnected: 'Disconnected', connecting: 'Connecting', awaitingTrust: 'Awaiting first-contact trust',
  connected: 'Connected', disconnecting: 'Disconnecting', failed: 'Failed',
};

export function SecurityWorkspace({ host }: { host: Host }) {
  const { hostname, port, authentication } = host.connection;
  // A configuration revisit starts a new local read; even an earlier same-config result
  // must not survive an intervening edit or host selection.
  return <EndpointTrust key={JSON.stringify([host.id, hostname, port, authentication])} host={host} />;
}

function EndpointTrust({ host }: { host: Host }) {
  const sessionQuery = useHostSession(host.id);
  const { hostname, port, authentication } = host.connection;
  const owner = JSON.stringify([host.id, hostname, port, authentication]);
  const generation = useRef(0);
  const inFlight = useRef(false);
  const [result, setResult] = useState<{ owner: string; trust: SshEndpointTrust | null } | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const read = useCallback((current: number) => {
    inFlight.current = true;
    void securityApi.get(host.id).then(
      (trust) => {
        if (generation.current !== current) return;
        const matches = trust.hostId === host.id && trust.hostname === hostname
          && trust.port === port && trust.authentication === authentication;
        setResult({ owner, trust: matches ? trust : null });
      },
      () => {
        if (generation.current === current) setResult({ owner, trust: null });
      },
    ).finally(() => {
      if (generation.current === current) {
        inFlight.current = false;
        setRefreshing(false);
      }
    });
  }, [host.id, hostname, port, authentication, owner]);

  useEffect(() => {
    read(++generation.current);
    return () => { generation.current += 1; };
  }, [read]);

  const currentResult = result?.owner === owner ? result : null;
  const trust = refreshing ? null : currentResult?.trust ?? null;
  const pending = refreshing || !currentResult;
  const unavailable = !pending && !trust;
  // Query can retain old data on error. Never infer state or verification from that data.
  const session = !sessionQuery.isError && sessionQuery.data?.hostId === host.id
    ? sessionQuery.data : null;
  const endpoint = hostname.includes(':') ? `[${hostname}]:${port}` : `${hostname}:${port}`;

  function refresh() {
    if (inFlight.current) return;
    setRefreshing(true);
    read(++generation.current);
  }

  return (
    <section aria-labelledby="security-title">
      <header className="page-heading">
        <div><div className="eyebrow">SSH TRUST</div><h1 id="security-title">Security</h1><p>{host.displayName}</p></div>
        <Button onClick={refresh} disabled={pending}>Refresh</Button>
      </header>
      {pending && <Spinner label="Reading local endpoint trust…" />}
      {unavailable && <Notice>Endpoint trust is unavailable. Refresh to retry.</Notice>}
      <div className="overview-columns">
      {trust && (
        <section className="panel" aria-labelledby="endpoint-trust-title">
          <div className="panel-heading"><h2 id="endpoint-trust-title">Endpoint trust</h2></div>
          <dl className="detail-list">
            <div><dt>Configured endpoint</dt><dd>{endpoint}</dd></div>
            <div><dt>Configured authentication</dt><dd>{authentication === 'password' ? 'Password' : 'Private key'}</dd></div>
            <div><dt>Endpoint pin</dt><dd>{trust.endpointPin ? 'Pinned locally' : 'Not pinned'}</dd></div>
            {trust.endpointPin && <>
              <div><dt>Algorithm</dt><dd>{trust.endpointPin.algorithm}</dd></div>
              <div><dt>SHA-256</dt><dd>{trust.endpointPin.sha256}</dd></div>
            </>}
          </dl>
          <p className="detail-list">Pins are stored locally for the configured hostname and port. They are not a fresh observation of the remote server.</p>
        </section>
      )}
      <section className="panel" aria-labelledby="ssh-session-title">
        <div className="panel-heading"><h2 id="ssh-session-title">Current SSH session</h2></div>
        <div className="detail-list">{session ? <>
          <p>Connection state: {connectionLabels[session.state]}</p>
          <p>{sessionStatus(session, trust)}</p>
          {session.error?.code === 'changedHostKey' && (
            <Notice>Connection blocked because the presented host key did not match the endpoint pin.</Notice>
          )}
        </> : <Notice tone="neutral">Current SSH session state is unavailable.</Notice>}</div>
      </section>
      </div>
    </section>
  );
}
