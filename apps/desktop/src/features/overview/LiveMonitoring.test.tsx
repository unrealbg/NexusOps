import { act, render, screen } from '@testing-library/react';
import type { HostMonitorSample } from '@nexusops/protocol';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ sample: vi.fn() }));
vi.mock('../../api/client', () => ({
  monitorApi: { sample: mocks.sample },
  applicationError: (error: unknown) =>
    typeof error === 'object' && error !== null && 'message' in error
      ? error
      : { code: 'connection', message: 'Monitoring failed.', hostKey: null },
}));

import { LiveMonitoring, MONITOR_HISTORY_LIMIT, MONITOR_INTERVAL_MS } from './LiveMonitoring';

const SESSION_A = '11111111-1111-4111-8111-111111111111';
const SESSION_B = '22222222-2222-4222-8222-222222222222';

function sample(overrides: Partial<HostMonitorSample> = {}): HostMonitorSample {
  return {
    hostId: 'host-a',
    hostSessionId: SESSION_A,
    observedAt: new Date().toISOString(),
    cpuUsagePercent: null,
    memoryTotalBytes: 8 * 1024 ** 3,
    memoryUsedBytes: 2 * 1024 ** 3,
    swapTotalBytes: 0,
    swapUsedBytes: 0,
    rootTotalBytes: 100 * 1024 ** 3,
    rootUsedBytes: 25 * 1024 ** 3,
    networkRxBytesPerSecond: null,
    networkTxBytesPerSecond: null,
    warnings: [],
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-25T12:00:00Z'));
});
afterEach(() => vi.useRealTimers());

describe('live monitoring lifecycle', () => {
  it('polls only while visible and connected, at completion-based five-second cadence', async () => {
    const first = deferred<HostMonitorSample>();
    mocks.sample
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue(sample({ cpuUsagePercent: 25 }));
    const view = render(
      <LiveMonitoring hostId="host-a" hostSessionId={SESSION_A} connected visible={false} />,
    );
    expect(mocks.sample).not.toHaveBeenCalled();
    view.rerender(<LiveMonitoring hostId="host-a" hostSessionId={SESSION_A} connected visible />);
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MONITOR_INTERVAL_MS * 3);
    });
    expect(mocks.sample).toHaveBeenCalledTimes(1);
    await act(async () => {
      first.resolve(sample());
      await Promise.resolve();
    });
    expect(screen.getAllByText('Warming up')).toHaveLength(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MONITOR_INTERVAL_MS);
    });
    expect(mocks.sample).toHaveBeenCalledTimes(2);
    expect(screen.getAllByText('25.0%')).toHaveLength(3);
    view.rerender(
      <LiveMonitoring hostId="host-a" hostSessionId={null} connected={false} visible />,
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MONITOR_INTERVAL_MS * 2);
    });
    expect(mocks.sample).toHaveBeenCalledTimes(2);
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
  });

  it('ignores a delayed old-session sample and starts new sessions with empty history', async () => {
    const old = deferred<HostMonitorSample>();
    mocks.sample
      .mockReturnValueOnce(old.promise)
      .mockImplementation(async (hostId, hostSessionId) =>
        sample({ hostId, hostSessionId, cpuUsagePercent: hostId === 'host-b' ? 7 : null }),
      );
    const view = render(
      <LiveMonitoring hostId="host-a" hostSessionId={SESSION_A} connected visible />,
    );
    view.rerender(<LiveMonitoring hostId="host-a" hostSessionId={SESSION_B} connected visible />);
    await act(async () => {
      old.resolve(sample({ cpuUsagePercent: 99 }));
      await Promise.resolve();
    });
    expect(screen.queryByText('99.0%')).not.toBeInTheDocument();
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('Not configured')).toBeInTheDocument();
    expect(screen.getAllByText('Warming up')).toHaveLength(2);
    expect(mocks.sample).toHaveBeenLastCalledWith('host-a', SESSION_B);
    view.rerender(<LiveMonitoring hostId="host-b" hostSessionId={SESSION_B} connected visible />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText('7.0%')).toBeInTheDocument();
    expect(screen.queryByText('99.0%')).not.toBeInTheDocument();
  });

  it('distinguishes unavailable, valid zero, stale data and enforces the history bound', async () => {
    let index = 0;
    mocks.sample.mockImplementation(async () =>
      sample({
        observedAt: index === 0 ? '2026-09-25T11:59:40Z' : new Date().toISOString(),
        cpuUsagePercent: index,
        networkRxBytesPerSecond: 0,
        networkTxBytesPerSecond: 0,
        memoryTotalBytes: null,
        memoryUsedBytes: null,
        warnings: [
          { code: 'discovery', message: 'memory monitoring is unavailable.', hostKey: null },
        ],
      }),
    );
    render(<LiveMonitoring hostId="host-a" hostSessionId={SESSION_A} connected visible />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText(/Stale · Updated 20 seconds ago/)).toBeInTheDocument();
    expect(screen.getByText('↓ 0 B/s')).toBeInTheDocument();
    expect(screen.getByText('↑ 0 B/s')).toBeInTheDocument();
    expect(screen.getByText('Not configured')).toBeInTheDocument();
    expect(screen.getAllByText('Unavailable').length).toBeGreaterThan(0);
    for (index = 1; index <= MONITOR_HISTORY_LIMIT + 3; index += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MONITOR_INTERVAL_MS);
      });
    }
    expect(mocks.sample).toHaveBeenCalledTimes(MONITOR_HISTORY_LIMIT + 4);
    for (const chart of document.querySelectorAll('.monitor-chart')) {
      expect(chart).toHaveAttribute('data-point-count', String(MONITOR_HISTORY_LIMIT));
    }
    expect(screen.getByText(`${(MONITOR_HISTORY_LIMIT + 3).toFixed(1)}%`)).toBeInTheDocument();
  });

  it('cleans up timers and does not publish after unmount', async () => {
    const pending = deferred<HostMonitorSample>();
    mocks.sample.mockReturnValue(pending.promise);
    const view = render(
      <LiveMonitoring hostId="host-a" hostSessionId={SESSION_A} connected visible />,
    );
    view.unmount();
    await act(async () => {
      pending.resolve(sample({ cpuUsagePercent: 88 }));
      await Promise.resolve();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MONITOR_INTERVAL_MS * 2);
    });
    expect(mocks.sample).toHaveBeenCalledTimes(1);
  });
});
