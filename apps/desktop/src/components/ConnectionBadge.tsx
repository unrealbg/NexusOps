import type { ConnectionState } from '@nexusops/protocol';
import { Badge } from '@nexusops/ui';

const states: Record<
  ConnectionState,
  { label: string; tone: 'neutral' | 'success' | 'warning' | 'danger' }
> = {
  disconnected: { label: 'Disconnected', tone: 'neutral' },
  connecting: { label: 'Connecting', tone: 'warning' },
  awaitingTrust: { label: 'Verify identity', tone: 'warning' },
  connected: { label: 'Connected', tone: 'success' },
  disconnecting: { label: 'Disconnecting', tone: 'warning' },
  failed: { label: 'Connection failed', tone: 'danger' },
};

export function ConnectionBadge({ state }: { state: ConnectionState }) {
  const { label, tone } = states[state];
  return <Badge tone={tone}>{label}</Badge>;
}
