/** Development-only visual test entry. Never imported by the production app/build. */
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Host, HostSession } from '@nexusops/protocol';
import App from '../App';
import { hostApi } from '../api/client';
import { useSelection } from '../state/selection';
import { connected, disconnected, host } from './fixtures';
import '@nexusops/ui/styles.css';
import '../styles.css';

if (!import.meta.env.DEV) throw new Error('Visual test entry is development-only.');
const mode = new URLSearchParams(location.search).get('state') ?? 'connected';
const hosts: Host[] =
  mode === 'empty'
    ? []
    : [
        host,
        {
          ...host,
          id: 'test-host-2',
          displayName: 'Test worker',
          connection: {
            ...host.connection,
            hostname: 'worker.example.test',
            authentication: 'privateKey',
          },
        },
      ];
const session: HostSession =
  mode === 'unknown' || mode === 'changed'
    ? {
        ...disconnected,
        state: mode === 'unknown' ? 'awaitingTrust' : 'failed',
        error: {
          code: mode === 'unknown' ? 'unknownHostKey' : 'changedHostKey',
          message: 'Visual test host-key challenge.',
          hostKey: {
            hostname: host.connection.hostname,
            port: 22,
            algorithm: 'ssh-ed25519',
            fingerprint: 'SHA256:UHEOcEvPnWsLfATpKNAVBkfrnzBZpcvQQtwLXDeOxlc',
            previousFingerprint:
              mode === 'changed' ? 'SHA256:ORhMQdlQzuYYNkDFefzHAMXlDoKRExqmqETftHrYAcg' : null,
          },
        },
      }
    : connected;
hostApi.list = async () => hosts;
hostApi.session = async (id) => (id === host.id ? session : { ...disconnected, hostId: id });
hostApi.connect = hostApi.reconnect = hostApi.disconnect = hostApi.refresh = async () => undefined;
hostApi.trust = async () => undefined;
hostApi.save = async () => {
  throw {
    code: 'policy',
    message: 'Saving is disabled in this visual test fixture.',
    hostKey: null,
  };
};
hostApi.delete = async () => undefined;
useSelection.setState({ hostId: mode === 'empty' || mode === 'list' ? null : host.id });
const root = document.getElementById('root');
if (!root) throw new Error('Missing visual test root.');
createRoot(root).render(
  <QueryClientProvider client={new QueryClient()}>
    <div style={{ padding: '5px 15px', color: '#edc780', background: '#342d21', fontSize: 11 }}>
      UI TEST FIXTURE · Simulated hosts and measurements · No SSH connection
    </div>
    <App />
  </QueryClientProvider>,
);
