import { describe, expect, it } from 'vitest';
import { bytes, percentage, uptime } from './format';

describe('discovery formatting', () => {
  it('keeps missing measurements distinct from real zeroes', () => {
    expect(bytes(null)).toBe('Unavailable');
    expect(bytes(0)).toBe('0 B');
    expect(bytes(1024 ** 3)).toBe('1 GiB');
    expect(uptime(null)).toBe('Unavailable');
    expect(uptime(0)).toBe('0m');
    expect(percentage(null, 100)).toBeNull();
    expect(percentage(0, 100)).toBe(0);
  });
  it('handles invalid totals and bounds meter values', () => {
    expect(percentage(20, 0)).toBeNull();
    expect(percentage(200, 100)).toBe(100);
    expect(percentage(-2, 100)).toBeNull();
    expect(bytes(NaN)).toBe('Unavailable');
    expect(uptime(90000)).toBe('1d 1h');
  });
});
