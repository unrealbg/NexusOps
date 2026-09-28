import { useCallback, useEffect, useRef, useState } from 'react';
import type { Host, HostKeyRotationPlan, HostSession, SshEndpointTrust } from '@nexusops/protocol';
import { Button, Modal, Notice, Spinner } from '@nexusops/ui';
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

function canonicalHostname(hostname: string): string | null {
  try {
    const authority = hostname.includes(':') ? `[${hostname}]` : hostname;
    return new URL(`ssh://${authority}`).hostname.replace(/^\[|\]$/g, '').replace(/\.$/, '').toLowerCase();
  } catch {
    return null;
  }
}

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
  const [rotation, setRotation] = useState<{ owner: string; plan: HostKeyRotationPlan } | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [executing, setExecuting] = useState(false);
  const [rotationNotice, setRotationNotice] = useState<string | null>(null);
  const rotationEpoch = useRef(0);
  const requestSequence = useRef(0);
  const activeRequest = useRef<number | null>(null);
  const executionActive = useRef(false);
  const rotated = useRef(false);

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
  const changed = session?.state === 'failed' && session.error?.code === 'changedHostKey'
    && session.error.hostKey?.previousFingerprint ? session.error.hostKey : null;
  const rotationOwner = changed ? JSON.stringify([
    owner, session?.hostSessionId, session?.error?.code, changed.hostname, changed.port,
    changed.algorithm, changed.fingerprint, changed.previousFingerprint,
  ]) : null;
  const liveRotation = rotation?.owner === rotationOwner ? rotation.plan : null;

  useEffect(() => {
    rotationEpoch.current += 1;
    activeRequest.current = null;
    const epoch = rotationEpoch.current;
    queueMicrotask(() => {
      if (rotationEpoch.current !== epoch) return;
      setRotation(null);
      setConfirmed(false);
      setPlanning(false);
      if (rotationOwner || !rotated.current) setRotationNotice(null);
      if (rotationOwner) rotated.current = false;
    });
    return () => { rotationEpoch.current += 1; activeRequest.current = null; };
  }, [rotationOwner]);

  function refresh() {
    if (inFlight.current) return;
    setRefreshing(true);
    read(++generation.current);
  }

  function refreshTrust() {
    setRefreshing(true);
    read(++generation.current);
  }

  async function reviewRotation() {
    if (!rotationOwner || activeRequest.current !== null || executionActive.current) return;
    const requestId = ++requestSequence.current;
    const epoch = rotationEpoch.current;
    activeRequest.current = requestId;
    setPlanning(true);
    setRotationNotice(null);
    try {
      const plan = await securityApi.planRotation(host.id);
      if (rotationEpoch.current !== epoch || activeRequest.current !== requestId) return;
      if (plan.hostId !== host.id || plan.hostname !== canonicalHostname(hostname)
          || plan.port !== port) {
        setRotationNotice('The rotation plan did not match this host. Review the connection again.');
        return;
      }
      setConfirmed(false);
      setRotation({ owner: rotationOwner, plan });
    } catch {
      if (rotationEpoch.current === epoch && activeRequest.current === requestId) {
        setRotationNotice('The key change could not be prepared. Reconnect and review it again.');
      }
    } finally {
      if (activeRequest.current === requestId) {
        activeRequest.current = null;
        setPlanning(false);
      }
    }
  }

  function closeRotation() {
    if (executionActive.current) return;
    setRotation(null);
    setConfirmed(false);
  }

  async function executeRotation() {
    if (!liveRotation || !confirmed || executionActive.current || !rotationOwner) return;
    executionActive.current = true;
    setExecuting(true);
    const epoch = rotationEpoch.current;
    try {
      await securityApi.executeRotation(host.id, liveRotation.id);
      if (rotationEpoch.current !== epoch) return;
      setRotation(null);
      setConfirmed(false);
      rotated.current = true;
      setRotationNotice('Local endpoint pin replaced. Reconnect normally to verify the new server key before authentication.');
      refreshTrust();
      void sessionQuery.refetch?.();
    } catch {
      if (rotationEpoch.current !== epoch) return;
      setRotation(null);
      setConfirmed(false);
      setRotationNotice('The rotation result needs review. Local endpoint trust is being reread; prepare a new plan only after checking the current pin.');
      refreshTrust();
      void sessionQuery.refetch?.();
    } finally {
      executionActive.current = false;
      if (rotationEpoch.current === epoch) setExecuting(false);
    }
  }

  return (
    <section aria-labelledby="security-title">
      <header className="page-heading">
        <div><div className="eyebrow">SSH TRUST</div><h1 id="security-title">Security</h1><p>{host.displayName}</p></div>
        <Button onClick={refresh} disabled={pending}>Refresh</Button>
      </header>
      {pending && <Spinner label="Reading local endpoint trust…" />}
      {unavailable && <Notice>Endpoint trust is unavailable. Refresh to retry.</Notice>}
      {rotationNotice && <Notice tone="warning">{rotationNotice}</Notice>}
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
          {session.error?.code === 'changedHostKey' && <Notice>
            Connection blocked because the presented host key did not match the endpoint pin.
            {changed && <div><Button onClick={() => void reviewRotation()} disabled={planning || executing}>
              Review key change
            </Button></div>}
          </Notice>}
        </> : <Notice tone="neutral">Current SSH session state is unavailable.</Notice>}</div>
      </section>
      </div>
      {liveRotation && <Modal title="Review SSH host-key change" onClose={closeRotation} wide>
        <p className="dialog-description">The presented key was observed during a connection attempt that NexusOps blocked because it did not match the stored endpoint pin. Verify the new fingerprint through an independent trusted channel before replacing the local pin.</p>
        <dl className="trust-details">
          <div><dt>Configured endpoint</dt><dd>{endpoint}</dd></div>
          <div><dt>Currently trusted algorithm</dt><dd>{liveRotation.currentFingerprint.algorithm}</dd></div>
          <div><dt>Currently trusted SHA-256</dt><dd className="fingerprint">{liveRotation.currentFingerprint.sha256}</dd></div>
          <div><dt>Presented during blocked connection algorithm</dt><dd>{liveRotation.presentedFingerprint.algorithm}</dd></div>
          <div><dt>Presented during blocked connection SHA-256</dt><dd className="fingerprint">{liveRotation.presentedFingerprint.sha256}</dd></div>
          <div><dt>Plan expires</dt><dd>{new Date(liveRotation.expiresAtUnixMs).toLocaleString()}</dd></div>
        </dl>
        <label className="field"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} disabled={executing} /> I independently verified the new fingerprint.</label>
        <div className="modal-actions">
          <Button onClick={closeRotation} disabled={executing}>Cancel</Button>
          <Button variant="danger" onClick={() => void executeRotation()} disabled={!confirmed || executing}>Replace endpoint pin</Button>
        </div>
      </Modal>}
    </section>
  );
}
