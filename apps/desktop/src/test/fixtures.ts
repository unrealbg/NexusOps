import type { DiscoverySnapshot, Host, HostSession } from '@nexusops/protocol';

export const host: Host = {
  id: 'test-host-1',
  displayName: 'Test gateway',
  connection: {
    hostname: 'gateway.example.test',
    port: 22,
    username: 'deploy',
    authentication: 'password',
  },
};
export const snapshot: DiscoverySnapshot = {
  hostname: 'gateway',
  os: 'Debian GNU/Linux',
  osVersion: '13',
  kernel: '6.12.0',
  architecture: 'x86_64',
  uptimeSeconds: 90000,
  loadOne: 0.42,
  memoryTotalBytes: 8 * 1024 ** 3,
  memoryUsedBytes: 2 * 1024 ** 3,
  rootTotalBytes: 100 * 1024 ** 3,
  rootUsedBytes: 30 * 1024 ** 3,
  observedAt: '2026-09-14T12:00:00Z',
  warnings: [],
};
export const disconnected: HostSession = {
  hostId: host.id,
  hostSessionId: null,
  state: 'disconnected',
  error: null,
  identity: null,
  capabilities: [],
  discovery: null,
};
export const connected: HostSession = {
  ...disconnected,
  state: 'connected',
  hostSessionId: '11111111-1111-4111-8111-111111111111',
  identity: {
    hostname: host.connection.hostname,
    fingerprint: { algorithm: 'ssh-ed25519', sha256: 'SHA256:test-fingerprint' },
  },
  discovery: snapshot,
  capabilities: [{ id: 'linux', available: true }],
};
