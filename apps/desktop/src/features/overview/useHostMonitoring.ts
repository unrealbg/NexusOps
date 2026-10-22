import { useEffect, useRef, useState } from 'react';
import type { HostMonitorSample } from '@nexusops/protocol';
import { monitorApi, applicationError } from '../../api/client';
import { percentage } from './format';

export const MONITOR_INTERVAL_MS = 5_000;
export const MONITOR_HISTORY_LIMIT = 120;
const STALE_AFTER_MS = 15_000;

// Share only work already in flight across visible presentations. Entries are
// removed at completion; no sample history or background sampler lives here.
const inFlight = new Map<string, Promise<HostMonitorSample>>();
function sampleOnce(hostId: string, hostSessionId: string) {
  const owner = `${hostId}:${hostSessionId}`;
  const current = inFlight.get(owner);
  if (current) return current;
  const pending = Promise.resolve().then(() => monitorApi.sample(hostId, hostSessionId)).finally(() => {
    if (inFlight.get(owner) === pending) inFlight.delete(owner);
  });
  inFlight.set(owner, pending);
  return pending;
}

export type MonitoringProps = {
  hostId: string;
  hostSessionId: string | null;
  connected: boolean;
  visible: boolean;
};

function metricWarning(sample: HostMonitorSample | undefined, name: string): boolean {
  return Boolean(sample?.warnings.some((warning) => warning.message.toLowerCase().includes(name)));
}

export function formatPercent(value: number | null): string {
  return value === null ? 'Unavailable' : `${value.toFixed(1)}%`;
}

function ageText(observedAt: string, now: number): string {
  const time = Date.parse(observedAt);
  if (!Number.isFinite(time)) return 'Update time unavailable';
  const seconds = Math.max(0, Math.floor((now - time) / 1000));
  return `Updated ${seconds} ${seconds === 1 ? 'second' : 'seconds'} ago`;
}

export function useHostMonitoring({ hostId, hostSessionId, connected, visible }: MonitoringProps) {
  const owner = `${hostId}:${hostSessionId ?? ''}`;
  const [timeline, setTimeline] = useState<{ owner: string; samples: HostMonitorSample[] }>({
    owner,
    samples: [],
  });
  const [requestError, setRequestError] = useState<{ owner: string; message: string } | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const generation = useRef(0);

  useEffect(() => {
    if (!connected || !visible || hostSessionId === null) return;
    const activeGeneration = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const poll = async () => {
      try {
        const sample = await sampleOnce(hostId, hostSessionId);
        if (stopped || generation.current !== activeGeneration) return;
        if (sample.hostId !== hostId || sample.hostSessionId !== hostSessionId) return;
        setTimeline((current) => ({
          owner,
          samples: [...(current.owner === owner ? current.samples : []), sample].slice(
            -MONITOR_HISTORY_LIMIT,
          ),
        }));
        setRequestError(null);
        setNow(Date.now());
      } catch (reason) {
        if (stopped || generation.current !== activeGeneration) return;
        setRequestError({ owner, message: applicationError(reason).message });
      }
      if (!stopped && generation.current === activeGeneration)
        timer = setTimeout(poll, MONITOR_INTERVAL_MS);
    };
    void Promise.resolve().then(() => { if (!stopped) void poll(); });
    return () => {
      stopped = true;
      generation.current += 1;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [connected, hostId, hostSessionId, owner, visible]);

  useEffect(() => {
    if (!connected || !visible) return;
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [connected, visible]);

  const visibleHistory = connected && timeline.owner === owner ? timeline.samples : [];
  const error = requestError?.owner === owner ? requestError.message : null;
  const sample = visibleHistory.at(-1);
  const age = sample ? now - Date.parse(sample.observedAt) : 0;
  const freshness = !connected
    ? 'Disconnected'
    : !sample
      ? error
        ? 'Monitoring unavailable'
        : 'Warming up'
      : age > STALE_AFTER_MS
        ? `Stale · ${ageText(sample.observedAt, now)}`
        : ageText(sample.observedAt, now);
  const cpuUnavailable = metricWarning(sample, 'cpu');
  const networkUnavailable = metricWarning(sample, 'network');
  const memoryUnavailable = metricWarning(sample, 'memory');
  const swapUnavailable = metricWarning(sample, 'swap');
  const diskUnavailable = metricWarning(sample, 'root filesystem');
  const memoryPercent = percentage(
    sample?.memoryUsedBytes ?? null,
    sample?.memoryTotalBytes ?? null,
  );
  const swapPercent = percentage(sample?.swapUsedBytes ?? null, sample?.swapTotalBytes ?? null);
  const diskPercent = percentage(sample?.rootUsedBytes ?? null, sample?.rootTotalBytes ?? null);

  return { visibleHistory, error, sample, age, freshness, cpuUnavailable, networkUnavailable,
    memoryUnavailable, swapUnavailable, diskUnavailable, memoryPercent, swapPercent, diskPercent };
}
