import { describe, expect, it } from 'vitest';
import { validateHost } from './validation';
import { host } from '../../test/fixtures';

describe('host form validation', () => {
  it('allows DNS and IPv6 endpoints without interpreting them as URLs', () => {
    expect(validateHost(host)).toBeNull();
    expect(
      validateHost({ ...host, connection: { ...host.connection, hostname: '2001:db8::1' } }),
    ).toBeNull();
  });
  it('rejects blank names, malformed endpoints and out-of-range ports', () => {
    expect(validateHost({ ...host, displayName: '  ' })).toContain('display name');
    expect(
      validateHost({ ...host, connection: { ...host.connection, hostname: 'ssh://example.test' } }),
    ).toContain('hostname');
    for (const port of [0, 65536, 22.2, NaN])
      expect(validateHost({ ...host, connection: { ...host.connection, port } })).toContain('port');
  });
});
