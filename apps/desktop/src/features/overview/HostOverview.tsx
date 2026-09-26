import { useMutation, useQueryClient } from '@tanstack/react-query';
import type { Host, HostKeyChallenge } from '@nexusops/protocol';
import { Button, Notice, Spinner } from '@nexusops/ui';
import { hostApi, applicationError } from '../../api/client';
import { hostKeys, useHostSession } from '../../api/queries';
import { ConnectionBadge } from '../../components/ConnectionBadge';
import { Icon } from '../../components/Icon';
import { DiscoveryDetails } from './DiscoveryDetails';
import { HostKeyNotice } from './HostKeyNotice';
import { LiveMonitoring } from './LiveMonitoring';

export function HostOverview({
  host,
  onEdit,
  onDelete,
}: {
  host: Host;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const queryClient = useQueryClient();
  const sessionQuery = useHostSession(host.id);
  const command = useMutation({
    mutationFn: (action: 'connect' | 'reconnect' | 'refresh') => hostApi[action](host.id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: hostKeys.session(host.id) }),
  });
  // Cancellation remains independently callable while a long connect/discovery is pending.
  const disconnect = useMutation({
    mutationFn: () => hostApi.disconnect(host.id),
    onSettled: () => {
      command.reset();
      return queryClient.invalidateQueries({ queryKey: hostKeys.session(host.id) });
    },
  });
  const trust = useMutation({
    mutationFn: async (challenge: HostKeyChallenge) => {
      await hostApi.trust(host.id, challenge);
      await hostApi.connect(host.id);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: hostKeys.session(host.id) }),
  });
  const session = sessionQuery.data;
  const state = session?.state ?? 'disconnected';
  const busy =
    command.isPending || trust.isPending || state === 'connecting' || state === 'disconnecting';
  const active = state === 'connected' || state === 'connecting' || state === 'awaitingTrust';
  const operationError = disconnect.error ?? trust.error ?? command.error;
  const error = session?.error ?? (operationError ? applicationError(operationError) : null);
  const keyError = error?.code === 'unknownHostKey' || error?.code === 'changedHostKey';

  return (
    <>
      <header className="page-heading">
        <div>
          <div className="eyebrow">HOST OVERVIEW</div>
          <h1>{host.displayName}</h1>
          <div className="host-subtitle">
            <span className="mono">
              {host.connection.username}@{host.connection.hostname}:{host.connection.port}
            </span>
            <span className="subtitle-separator">/</span>
            <ConnectionBadge state={state} />
          </div>
        </div>
        <div className="heading-actions">
          <Button variant="ghost" onClick={onEdit} disabled={busy || active}>
            Edit host
          </Button>
          <Button variant="ghost" onClick={onDelete} disabled={busy || active}>
            Delete
          </Button>
          {active || busy ? (
            <Button
              disabled={disconnect.isPending || state === 'disconnecting'}
              onClick={() => disconnect.mutate()}
            >
              {state === 'connecting' || state === 'awaitingTrust'
                ? 'Cancel connection'
                : 'Disconnect'}
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={
                sessionQuery.isPending || (state === 'failed' && error?.code === 'changedHostKey')
              }
              onClick={() => {
                trust.reset();
                command.mutate('connect');
              }}
            >
              <Icon name="terminal" size={16} />
              Connect
            </Button>
          )}
        </div>
      </header>
      <div className="section-tabs">
        <span className="section-tab">Overview</span>
        <div className="section-tabs-right">
          {state === 'connected' && (
            <>
              <Button variant="ghost" disabled={busy} onClick={() => command.mutate('reconnect')}>
                Reconnect
              </Button>
              <Button variant="ghost" disabled={busy} onClick={() => command.mutate('refresh')}>
                <Icon name="refresh" size={14} />
                {command.isPending ? 'Refreshing…' : 'Refresh'}
              </Button>
            </>
          )}
        </div>
      </div>
      {sessionQuery.isPending && (
        <div className="state-panel">
          <Spinner label="Reading connection state…" />
        </div>
      )}
      {sessionQuery.isError && (
        <Notice>
          {applicationError(sessionQuery.error).message}
          <div className="notice-action">
            <Button
              onClick={() => {
                void sessionQuery.refetch();
              }}
            >
              Retry
            </Button>
          </div>
        </Notice>
      )}
      {error && keyError && (
        <HostKeyNotice
          key={`${error.code}:${error.hostKey?.fingerprint}`}
          error={error}
          busy={trust.isPending}
          onTrust={(challenge) => {
            command.reset();
            trust.mutate(challenge);
          }}
        />
      )}
      {error && !keyError && error.code !== 'cancelled' && (
        <Notice>
          <strong>Unable to complete the request.</strong> {error.message}
        </Notice>
      )}
      {state === 'connecting' && (
        <div className="state-panel">
          <Spinner label="Connecting and discovering this host…" />
          <p>Verifying host identity, then collecting read-only system information.</p>
        </div>
      )}
      {state === 'disconnecting' && (
        <div className="state-panel">
          <Spinner label="Closing the SSH session…" />
        </div>
      )}
      {session && (state === 'connected' || state === 'disconnected' || state === 'failed') && (
        <LiveMonitoring
          hostId={host.id}
          hostSessionId={session.hostSessionId}
          connected={state === 'connected'}
          visible
        />
      )}
      {state === 'connected' &&
        session &&
        (session.discovery ? (
          <DiscoveryDetails discovery={session.discovery} session={session} />
        ) : (
          <div className="state-panel">
            <Spinner label="Discovering system information…" />
          </div>
        ))}
      {(state === 'disconnected' || state === 'failed') && !sessionQuery.isPending && !keyError && (
        <section className="connection-empty">
          <span className="large-icon">
            <Icon name="server" size={30} />
          </span>
          <h2>{state === 'failed' ? 'Connection needs attention' : 'Ready when you are'}</h2>
          <p>
            {state === 'failed'
              ? 'Review the connection details and try again.'
              : 'Connect to discover this host’s system information and available capabilities.'}
          </p>
          <div className="connection-facts">
            <span>
              <Icon name="security" size={15} />
              Verified SSH identity
            </span>
            <span>
              <Icon name="overview" size={15} />
              Read-only discovery
            </span>
          </div>
        </section>
      )}
    </>
  );
}
