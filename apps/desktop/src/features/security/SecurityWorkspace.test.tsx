import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Host, HostKeyRotationPlan, HostSession, SshEndpointTrust } from '@nexusops/protocol';
import { host, disconnected, connected } from '../../test/fixtures';

const mocks = vi.hoisted(() => ({ get: vi.fn(), planRotation: vi.fn(), executeRotation: vi.fn(), refetch: vi.fn(), session: null as HostSession | null, isError: false }));
vi.mock('../../api/client', () => ({ securityApi: { get: mocks.get, planRotation: mocks.planRotation, executeRotation: mocks.executeRotation } }));
vi.mock('../../api/queries', () => ({ useHostSession: () => ({ data: mocks.session, isError: mocks.isError, refetch: mocks.refetch }) }));
import { SecurityWorkspace } from './SecurityWorkspace';

const pin = { algorithm: 'ssh-ed25519', sha256: `SHA256:${'A'.repeat(43)}` };
const verifiedText = 'Verified against endpoint pin at session establishment';
const presented = { algorithm: 'ecdsa-sha2-nistp256', sha256: `SHA256:${'B'.repeat(43)}` };
function changedSession(): HostSession {
  return { ...disconnected, state: 'failed', error: { code: 'changedHostKey', message: 'sensitive challenge text', hostKey: {
    hostname: host.connection.hostname, port: host.connection.port, algorithm: presented.algorithm,
    fingerprint: presented.sha256, previousFingerprint: pin.sha256,
  } } };
}
function rotationPlan(): HostKeyRotationPlan {
  return { id: 'rotation-1', hostId: host.id, hostname: host.connection.hostname, port: host.connection.port,
    currentFingerprint: pin, presentedFingerprint: presented, expiresAtUnixMs: Date.UTC(2026, 8, 28, 12, 5) };
}
function trust(h = host, endpointPin: SshEndpointTrust['endpointPin'] = pin): SshEndpointTrust {
  return { hostId: h.id, hostname: h.connection.hostname, port: h.connection.port, authentication: h.connection.authentication, endpointPin };
}
function active(): HostSession {
  return { ...connected, identity: { hostname: host.connection.hostname, fingerprint: pin } };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  mocks.get.mockReset().mockResolvedValue(trust());
  mocks.planRotation.mockReset().mockResolvedValue(rotationPlan());
  mocks.executeRotation.mockReset().mockResolvedValue(undefined);
  mocks.refetch.mockReset().mockResolvedValue(undefined);
  mocks.session = disconnected;
  mocks.isError = false;
});

describe('local SSH endpoint trust', () => {
  it('reads an offline unpinned endpoint and shows only configured password metadata', async () => {
    mocks.get.mockResolvedValue(trust(host, null));
    render(<SecurityWorkspace host={host} />);
    expect(await screen.findByText('Not pinned')).toBeInTheDocument();
    expect(screen.getByText('Configured authentication')).toBeInTheDocument();
    expect(screen.getByText('Password')).toBeInTheDocument();
    expect(screen.getByText('gateway.example.test:22')).toBeInTheDocument();
    expect(screen.getByText('No active verified session')).toBeInTheDocument();
    expect(screen.queryByText(/Credential (available|valid|verified)/)).not.toBeInTheDocument();
    expect(screen.getAllByRole('button')).toHaveLength(1);
    expect(mocks.get).toHaveBeenCalledExactlyOnceWith(host.id);
  });

  it('shows a local pin, configured private-key method and bracketed IPv6 endpoint', async () => {
    const ipv6: Host = { ...host, connection: { ...host.connection, hostname: '2001:db8::1', authentication: 'privateKey' } };
    mocks.get.mockResolvedValue(trust(ipv6));
    render(<SecurityWorkspace host={ipv6} />);
    expect(await screen.findByText('Pinned locally')).toBeInTheDocument();
    expect(screen.getByText('[2001:db8::1]:22')).toBeInTheDocument();
    expect(screen.getByText('Private key')).toBeInTheDocument();
    expect(screen.getByText(pin.algorithm)).toBeInTheDocument();
    expect(screen.getByText(pin.sha256)).toBeInTheDocument();
    expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
  });

  it('renders even hostile text as text, never HTML', async () => {
    mocks.get.mockResolvedValue(trust(host, { algorithm: '<img src=x>', sha256: '<script>bad()</script>' }));
    render(<SecurityWorkspace host={host} />);
    expect(await screen.findByText('<img src=x>')).toBeInTheDocument();
    expect(screen.getByText('<script>bad()</script>')).toBeInTheDocument();
    expect(document.querySelector('img, script')).toBeNull();
  });

  it.each([
    ['disconnected', 'No active verified session'],
    ['disconnecting', 'No active verified session'],
    ['failed', 'No active verified session'],
    ['connecting', 'Host-key verification in progress'],
    ['awaitingTrust', 'Awaiting first-contact trust'],
  ] as const)('never verifies a %s session even with retained identity', async (state, message) => {
    mocks.session = { ...active(), state };
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    expect(screen.getByText(message)).toBeInTheDocument();
    expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
  });

  it('keeps unknown first-contact state distinct from not pinned', async () => {
    mocks.session = { ...disconnected, state: 'awaitingTrust' };
    mocks.get.mockResolvedValue(trust(host, null));
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Not pinned');
    expect(screen.getByText('Awaiting first-contact trust')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Trust/ })).not.toBeInTheDocument();
  });

  it('verifies only a matching connected session and removes that claim after remote closure', async () => {
    mocks.session = active();
    const view = render(<SecurityWorkspace host={host} />);
    expect(await screen.findByText(verifiedText)).toBeInTheDocument();
    // get_session retains identity while reconciling a closed transport to Failed.
    mocks.session = { ...active(), state: 'failed', hostSessionId: null, error: { code: 'connection', message: 'Connection closed.', hostKey: null } };
    view.rerender(<SecurityWorkspace host={host} />);
    expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
    expect(screen.getByText('Connection state: Failed')).toBeInTheDocument();
    expect(screen.getByText('No active verified session')).toBeInTheDocument();
    expect(screen.getByText(pin.sha256)).toBeInTheDocument();
    expect(mocks.get).toHaveBeenCalledTimes(1);
  });

  it.each(['sessionId', 'identity', 'hostname', 'algorithm', 'fingerprint', 'pin'] as const)(
    'fails closed for connected metadata with missing/mismatched %s', async (field) => {
      const session = active();
      if (field === 'sessionId') session.hostSessionId = null;
      if (field === 'identity') session.identity = null;
      if (field === 'hostname') session.identity = { ...session.identity!, hostname: 'other.example' };
      if (field === 'algorithm') session.identity = { ...session.identity!, fingerprint: { ...pin, algorithm: 'other' } };
      if (field === 'fingerprint') session.identity = { ...session.identity!, fingerprint: { ...pin, sha256: `SHA256:${'B'.repeat(43)}` } };
      if (field === 'pin') mocks.get.mockResolvedValue(trust(host, null));
      mocks.session = session;
      render(<SecurityWorkspace host={host} />);
      await waitFor(() => expect(screen.queryByText('Reading local endpoint trust…')).not.toBeInTheDocument());
      expect(screen.getByText(/Session verification unavailable/)).toBeInTheDocument();
      expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
    },
  );

  it('reports changed-key blocking without exposing the challenged key before explicit Review', async () => {
    mocks.session = changedSession();
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    expect(screen.getByText('Connection blocked because the presented host key did not match the endpoint pin.')).toBeInTheDocument();
    expect(screen.getByText('No active verified session')).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(presented.sha256);
    expect(document.body.textContent).not.toContain('sensitive challenge text');
    expect(screen.getByRole('button', { name: 'Review key change' })).toBeInTheDocument();
    expect(mocks.planRotation).not.toHaveBeenCalled();
  });

  it.each(['query error', 'wrong host', 'missing data'])(
    'does not infer disconnected or verified from %s', async (failure) => {
      mocks.session = active();
      if (failure === 'query error') mocks.isError = true;
      if (failure === 'wrong host') mocks.session.hostId = 'other';
      if (failure === 'missing data') mocks.session = null;
      render(<SecurityWorkspace host={host} />);
      await screen.findByText('Pinned locally');
      expect(screen.getByText('Current SSH session state is unavailable.')).toBeInTheDocument();
      expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
      expect(screen.queryByText('Connection state: Disconnected')).not.toBeInTheDocument();
    },
  );

  it('manual Refresh is exact-once while pending; failure removes prior pin and verification', async () => {
    mocks.session = active();
    const next = deferred<SshEndpointTrust>();
    mocks.get.mockResolvedValueOnce(trust()).mockReturnValueOnce(next.promise).mockResolvedValueOnce(trust(host, null));
    render(<SecurityWorkspace host={host} />);
    await screen.findByText(verifiedText);
    const button = screen.getByRole('button', { name: 'Refresh' });
    await userEvent.click(button);
    await userEvent.click(button);
    expect(mocks.get).toHaveBeenCalledTimes(2);
    expect(button).toBeDisabled();
    expect(screen.queryByText(pin.sha256)).not.toBeInTheDocument();
    await act(async () => next.reject(new Error('private diagnostic')));
    expect(await screen.findByText('Endpoint trust is unavailable. Refresh to retry.')).toBeInTheDocument();
    expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
    expect(screen.queryByText('Not pinned')).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain('private diagnostic');
    await userEvent.click(button);
    expect(await screen.findByText('Not pinned')).toBeInTheDocument();
    expect(mocks.get).toHaveBeenCalledTimes(3);
  });

  it.each(['hostId', 'hostname', 'port', 'authentication'] as const)(
    'ignores a delayed response owned by old %s and re-reads on configuration revisit', async (field) => {
      const old = deferred<SshEndpointTrust>();
      const nextHost: Host = { ...host, connection: { ...host.connection } };
      if (field === 'hostId') nextHost.id = 'other';
      if (field === 'hostname') nextHost.connection.hostname = 'other.example';
      if (field === 'port') nextHost.connection.port = 2222;
      if (field === 'authentication') nextHost.connection.authentication = 'privateKey';
      mocks.get.mockReturnValueOnce(old.promise).mockResolvedValueOnce(trust(nextHost, null));
      const view = render(<SecurityWorkspace host={host} />);
      view.rerender(<SecurityWorkspace host={nextHost} />);
      await screen.findByText('Not pinned');
      await act(async () => old.resolve(trust()));
      expect(screen.queryByText(pin.sha256)).not.toBeInTheDocument();
      const revisit = deferred<SshEndpointTrust>();
      mocks.get.mockReturnValueOnce(revisit.promise);
      view.rerender(<SecurityWorkspace host={host} />);
      expect(screen.queryByText('Not pinned')).not.toBeInTheDocument();
      expect(screen.getByText('Reading local endpoint trust…')).toBeInTheDocument();
      await act(async () => revisit.resolve(trust()));
      expect(screen.getByText(pin.sha256)).toBeInTheDocument();
      expect(mocks.get).toHaveBeenCalledTimes(3);
    },
  );

  it.each(['hostId', 'hostname', 'port', 'authentication'] as const)('rejects returned %s mismatch', async (field) => {
    const wrong = trust();
    if (field === 'hostId') wrong.hostId = 'wrong';
    if (field === 'hostname') wrong.hostname = 'wrong.example';
    if (field === 'port') wrong.port = 2222;
    if (field === 'authentication') wrong.authentication = 'privateKey';
    mocks.get.mockResolvedValue(wrong);
    mocks.session = active();
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Endpoint trust is unavailable. Refresh to retry.');
    expect(screen.queryByText(pin.sha256)).not.toBeInTheDocument();
    expect(screen.queryByText(verifiedText)).not.toBeInTheDocument();
    expect(screen.queryByText('Not pinned')).not.toBeInTheDocument();
  });

  it.each(['resolve', 'reject'] as const)('ignores %s after unmount without affecting a new view', async (outcome) => {
    const old = deferred<SshEndpointTrust>();
    mocks.get.mockReturnValueOnce(old.promise).mockResolvedValueOnce(trust(host, null));
    const first = render(<SecurityWorkspace host={host} />);
    first.unmount();
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Not pinned');
    await act(async () => { if (outcome === 'resolve') old.resolve(trust()); else old.reject(new Error('stale')); });
    expect(screen.getByText('Not pinned')).toBeInTheDocument();
    expect(screen.queryByText(pin.sha256)).not.toBeInTheDocument();
    expect(screen.queryByText(/Endpoint trust is unavailable/)).not.toBeInTheDocument();
  });

  it('adds no pin polling; session query updates do not trigger trust reads', async () => {
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    vi.useFakeTimers();
    try {
      await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
      expect(mocks.get).toHaveBeenCalledTimes(1);
    } finally { vi.useRealTimers(); }
  });
});

describe('guided local host-key rotation', () => {
  it.each(['disconnected', 'connecting', 'awaitingTrust', 'connected', 'ordinary failure', 'query error'])(
    'does not offer Review for %s', async (state) => {
      mocks.session = state === 'ordinary failure'
        ? { ...changedSession(), error: { code: 'connection', message: 'failure', hostKey: null } }
        : state === 'query error' ? changedSession() : { ...disconnected, state: state as HostSession['state'] };
      mocks.isError = state === 'query error';
      render(<SecurityWorkspace host={host} />);
      await screen.findByText('Pinned locally');
      expect(screen.queryByRole('button', { name: 'Review key change' })).not.toBeInTheDocument();
    },
  );

  it('requests only HostId, displays blocked-handshake evidence, and requires explicit confirmation', async () => {
    mocks.session = changedSession();
    const user = userEvent.setup();
    render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    expect(mocks.planRotation).toHaveBeenCalledExactlyOnceWith(host.id);
    const dialog = await screen.findByRole('dialog', { name: 'Review SSH host-key change' });
    expect(dialog.textContent).toContain('gateway.example.test:22');
    expect(dialog.textContent).toContain(pin.algorithm);
    expect(dialog.textContent).toContain(pin.sha256);
    expect(dialog.textContent).toContain(presented.algorithm);
    expect(dialog.textContent).toContain(presented.sha256);
    expect(dialog.textContent).toContain('Plan expires');
    expect(dialog.textContent).toContain('observed during a connection attempt that NexusOps blocked');
    expect(within(dialog).getByRole('button', { name: 'Replace endpoint pin' })).toBeDisabled();
    expect(within(dialog).queryByRole('button', { name: /Copy|Export/ })).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole('checkbox', { name: 'I independently verified the new fingerprint.' }));
    expect(within(dialog).getByRole('button', { name: 'Replace endpoint pin' })).toBeEnabled();
    expect(mocks.executeRotation).not.toHaveBeenCalled();
  });

  it('rejects plan owner mismatches and ignores delayed response after session turnover', async () => {
    mocks.session = changedSession();
    const user = userEvent.setup();
    mocks.planRotation.mockResolvedValueOnce({ ...rotationPlan(), hostname: 'different.example' });
    const view = render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    expect(await screen.findByText('The rotation plan did not match this host. Review the connection again.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const pending = deferred<HostKeyRotationPlan>();
    mocks.planRotation.mockReturnValueOnce(pending.promise);
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    mocks.session = disconnected;
    view.rerender(<SecurityWorkspace host={host} />);
    await act(async () => pending.resolve(rotationPlan()));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mocks.executeRotation).not.toHaveBeenCalled();
  });

  it('prevents overlapping Review requests and drops a plan after challenge or host replacement', async () => {
    mocks.session = changedSession();
    const pending = deferred<HostKeyRotationPlan>();
    mocks.planRotation.mockReturnValueOnce(pending.promise);
    const user = userEvent.setup();
    const view = render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    const review = screen.getByRole('button', { name: 'Review key change' });
    await user.click(review);
    await user.click(review);
    expect(mocks.planRotation).toHaveBeenCalledExactlyOnceWith(host.id);
    expect(review).toBeDisabled();
    await act(async () => pending.resolve(rotationPlan()));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    const replacement = changedSession();
    replacement.error!.hostKey!.fingerprint = `SHA256:${'C'.repeat(43)}`;
    mocks.session = replacement;
    view.rerender(<SecurityWorkspace host={host} />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    mocks.session = changedSession();
    view.rerender(<SecurityWorkspace host={host} />);
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    const other: Host = { ...host, id: 'different-host' };
    mocks.get.mockResolvedValueOnce(trust(other));
    mocks.session = { ...changedSession(), hostId: other.id };
    view.rerender(<SecurityWorkspace host={other} />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mocks.executeRotation).not.toHaveBeenCalled();
  });

  it('executes one HostId/PlanId mutation, closes the modal and rereads local trust without reconnect', async () => {
    mocks.session = changedSession();
    const pending = deferred<void>();
    mocks.executeRotation.mockReturnValueOnce(pending.promise);
    mocks.get.mockResolvedValueOnce(trust()).mockResolvedValueOnce(trust(host, presented));
    const user = userEvent.setup();
    const view = render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('checkbox'));
    const replace = within(dialog).getByRole('button', { name: 'Replace endpoint pin' });
    await user.click(replace);
    await user.click(replace);
    expect(mocks.executeRotation).toHaveBeenCalledExactlyOnceWith(host.id, rotationPlan().id);
    await act(async () => pending.resolve());
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(mocks.get).toHaveBeenCalledTimes(2);
    expect(mocks.refetch).toHaveBeenCalledTimes(1);
    mocks.session = disconnected;
    view.rerender(<SecurityWorkspace host={host} />);
    expect(screen.getByText('No active verified session')).toBeInTheDocument();
    expect(screen.getByText(/Local endpoint pin replaced\. Reconnect normally/)).toBeInTheDocument();
    expect(screen.getByText(presented.sha256)).toBeInTheDocument();
    expect(mocks.planRotation).toHaveBeenCalledTimes(1);
  });

  it('keeps successful mutation distinct from a failed trust reread', async () => {
    mocks.session = changedSession();
    mocks.get.mockResolvedValueOnce(trust()).mockRejectedValueOnce(new Error('private storage diagnostic'));
    const user = userEvent.setup();
    const view = render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    await user.click(within(await screen.findByRole('dialog')).getByRole('checkbox'));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Replace endpoint pin' }));
    expect(await screen.findByText('Endpoint trust is unavailable. Refresh to retry.')).toBeInTheDocument();
    mocks.session = disconnected;
    view.rerender(<SecurityWorkspace host={host} />);
    expect(screen.getByText(/Local endpoint pin replaced\. Reconnect normally/)).toBeInTheDocument();
    expect(screen.queryByText(pin.sha256)).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain('private storage diagnostic');
  });

  it('consumes a failed execution locally and invalidates an open plan on challenge, query or host change', async () => {
    mocks.session = changedSession();
    mocks.executeRotation.mockRejectedValueOnce(new Error('private failure'));
    const user = userEvent.setup();
    const view = render(<SecurityWorkspace host={host} />);
    await screen.findByText('Pinned locally');
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    await user.click(within(await screen.findByRole('dialog')).getByRole('checkbox'));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Replace endpoint pin' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(screen.getByText(/rotation result needs review/)).toBeInTheDocument();
    expect(document.body.textContent).not.toContain('private failure');
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    mocks.isError = true;
    view.rerender(<SecurityWorkspace host={host} />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    mocks.isError = false;
    mocks.session = changedSession();
    view.rerender(<SecurityWorkspace host={host} />);
    await user.click(screen.getByRole('button', { name: 'Review key change' }));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    const next: Host = { ...host, connection: { ...host.connection, port: 2222 } };
    mocks.get.mockResolvedValueOnce(trust(next));
    view.rerender(<SecurityWorkspace host={next} />);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mocks.executeRotation).toHaveBeenCalledTimes(1);
  });
});
