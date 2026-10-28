import { StrictMode } from 'react';
import { act, render, screen } from '@testing-library/react';
import type { HostMonitorSample } from '@nexusops/protocol';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CompactMonitorStrip } from './CompactMonitorStrip';
import { LiveMonitoring } from './LiveMonitoring';

const mocks = vi.hoisted(() => ({ sample: vi.fn() }));
vi.mock('../../api/client', () => ({
  monitorApi: { sample: mocks.sample },
  applicationError: () => ({ message: 'Monitoring failed.' }),
}));

const props = { hostId: 'host-a', hostSessionId: 'session-a', connected: true, visible: true };
function sample(overrides: Partial<HostMonitorSample> = {}): HostMonitorSample {
  return { hostId: 'host-a', hostSessionId: 'session-a', observedAt: new Date().toISOString(),
    cpuUsagePercent: 0, memoryUsedBytes: 0, memoryTotalBytes: 100,
    rootUsedBytes: 0, rootTotalBytes: 100, swapUsedBytes: 0, swapTotalBytes: 0,
    networkRxBytesPerSecond: 0, networkTxBytesPerSecond: 0, warnings: [], ...overrides };
}
async function flush() { await act(async () => { await Promise.resolve(); }); }
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-22T12:00:00Z'));
  mocks.sample.mockReset().mockResolvedValue(sample());
});
afterEach(() => vi.useRealTimers());

describe('compact monitoring ownership and cadence', () => {
  it('shares an in-flight Overview observation with Combined without a second request or retained shared history', async () => {
    let finish!: (value: HostMonitorSample) => void;
    mocks.sample.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const view = render(<LiveMonitoring {...props} />);
    await flush();
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    view.rerender(<CompactMonitorStrip {...props} />);
    await flush();
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await act(async () => { finish(sample({ cpuUsagePercent: 17 })); });
    expect(screen.getByText(/^CPU/)).toHaveTextContent('17.0%');
    view.rerender(<LiveMonitoring {...props} />);
    await flush();
    expect(mocks.sample).toHaveBeenCalledTimes(2);
    expect(document.querySelector('.monitor-chart')).toHaveAttribute('data-point-count', '1');
  });

  it('starts once under StrictMode and never starts when hidden or disconnected', async () => {
    const view = render(<StrictMode><CompactMonitorStrip {...props} visible={false} /></StrictMode>);
    await flush();
    expect(mocks.sample).not.toHaveBeenCalled();
    expect(screen.queryByRole('region')).not.toBeInTheDocument();
    view.rerender(<StrictMode><CompactMonitorStrip {...props} connected={false} /></StrictMode>);
    await flush();
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
    expect(mocks.sample).not.toHaveBeenCalled();
    view.rerender(<StrictMode><CompactMonitorStrip {...props} /></StrictMode>);
    await flush();
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    expect(mocks.sample).toHaveBeenCalledWith('host-a', 'session-a');
  });

  it('suppresses the discarded initial StrictMode request', async () => {
    render(<StrictMode><CompactMonitorStrip {...props} /></StrictMode>);
    await flush();
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await advance(4_999);
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await advance(1);
    expect(mocks.sample).toHaveBeenCalledTimes(2);
  });

  it('waits five seconds after completion and stops on hide and unmount', async () => {
    let finish!: (value: HostMonitorSample) => void;
    mocks.sample.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const view = render(<CompactMonitorStrip {...props} />);
    await flush();
    await advance(20_000);
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await act(async () => { finish(sample()); });
    await advance(4_999);
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await advance(1);
    expect(mocks.sample).toHaveBeenCalledTimes(2);
    view.rerender(<CompactMonitorStrip {...props} visible={false} />);
    await advance(20_000);
    expect(mocks.sample).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('distinguishes valid zeros, unavailable metrics and stale observations', async () => {
    mocks.sample.mockResolvedValue(sample({ observedAt: '2026-10-22T11:59:40Z', memoryUsedBytes: null,
      warnings: [{ code: 'discovery', message: 'memory unavailable', hostKey: null }] }));
    render(<CompactMonitorStrip {...props} />);
    await flush();
    expect(screen.getByText(/Stale · Updated 20 seconds ago/)).toBeInTheDocument();
    expect(screen.getByText(/^CPU/)).toHaveTextContent('CPU 0.0%');
    expect(screen.getByText(/^RAM/)).toHaveTextContent('RAM Unavailable');
    expect(screen.getByText(/^Root/)).toHaveTextContent('Root 0.0%');
    expect(screen.getByText(/^RX/)).toHaveTextContent('RX 0 B/s');
    expect(screen.getByText(/^TX/)).toHaveTextContent('TX 0 B/s');
  });

  it('reports missing first measurements separately from failures and recovers', async () => {
    mocks.sample.mockRejectedValueOnce(new Error('synthetic failure')).mockResolvedValueOnce(sample({ cpuUsagePercent: null, networkRxBytesPerSecond: null }));
    render(<CompactMonitorStrip {...props} />);
    await flush();
    expect(screen.getByText('Monitoring unavailable')).toBeInTheDocument();
    expect(screen.getByText('Monitoring failed.')).toBeInTheDocument();
    await advance(5_000);
    expect(screen.getByText(/^CPU/)).toHaveTextContent('Warming up');
    expect(screen.getByText(/^RX/)).toHaveTextContent('Warming up');
    expect(screen.queryByText('Monitoring failed.')).not.toBeInTheDocument();
  });

  it('never publishes old host/session responses and clears metrics on disconnect', async () => {
    let finish!: (value: HostMonitorSample) => void;
    mocks.sample.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }))
      .mockImplementation(async (hostId, hostSessionId) => sample({ hostId, hostSessionId, cpuUsagePercent: 7 }));
    const view = render(<CompactMonitorStrip {...props} />);
    await flush();
    view.rerender(<CompactMonitorStrip {...props} hostId="host-b" hostSessionId="session-b" />);
    await flush();
    await act(async () => { finish(sample({ cpuUsagePercent: 99 })); });
    expect(screen.getByText(/^CPU/)).toHaveTextContent('7.0%');
    expect(screen.queryByText('99.0%')).not.toBeInTheDocument();
    view.rerender(<CompactMonitorStrip {...props} hostId="host-b" hostSessionId="session-b" connected={false} />);
    await advance(10_000);
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
    expect(screen.queryByText('7.0%')).not.toBeInTheDocument();
    expect(mocks.sample).toHaveBeenCalledTimes(2);
  });

  it('ignores mismatched ownership and late completions after unmount', async () => {
    mocks.sample.mockResolvedValue(sample({ hostSessionId: 'wrong-session', cpuUsagePercent: 91 }));
    const view = render(<CompactMonitorStrip {...props} />);
    await flush();
    expect(screen.queryByText('91.0%')).not.toBeInTheDocument();
    expect(screen.getByText(/^CPU/)).toHaveTextContent('Warming up');
    view.unmount();
    await advance(20_000);
    expect(mocks.sample).toHaveBeenCalledTimes(1);
  });
});
