import { useEffect, useMemo, useRef, useState } from 'react';
import type { HostMonitorSample } from '@nexusops/protocol';
import { monitorApi, applicationError } from '../../api/client';
import { bytes, bytesPerSecond, percentage } from './format';

export const MONITOR_INTERVAL_MS = 5_000;
export const MONITOR_HISTORY_LIMIT = 120;
const STALE_AFTER_MS = 15_000;

type Props = {
  hostId: string;
  hostSessionId: string | null;
  connected: boolean;
  visible: boolean;
};

function metricWarning(sample: HostMonitorSample | undefined, name: string): boolean {
  return Boolean(sample?.warnings.some((warning) => warning.message.toLowerCase().includes(name)));
}

function formatPercent(value: number | null): string {
  return value === null ? 'Unavailable' : `${value.toFixed(1)}%`;
}

function ageText(observedAt: string, now: number): string {
  const time = Date.parse(observedAt);
  if (!Number.isFinite(time)) return 'Update time unavailable';
  const seconds = Math.max(0, Math.floor((now - time) / 1000));
  return `Updated ${seconds} ${seconds === 1 ? 'second' : 'seconds'} ago`;
}

function Sparkline({ values }: { values: Array<number | null> }) {
  const segments = useMemo(() => {
    const finite = values.filter(
      (value): value is number => value !== null && Number.isFinite(value),
    );
    if (finite.length < 2) return [];
    const min = Math.min(...finite);
    const max = Math.max(...finite);
    const range = Math.max(max - min, 1);
    const width = Math.max(values.length - 1, 1);
    const lines: string[] = [];
    let points: string[] = [];
    values.forEach((value, index) => {
      if (value === null || !Number.isFinite(value)) {
        if (points.length > 1) lines.push(points.join(' '));
        points = [];
        return;
      }
      points.push(`${(index / width) * 100},${28 - ((value - min) / range) * 24}`);
    });
    if (points.length > 1) lines.push(points.join(' '));
    return lines;
  }, [values]);
  return (
    <svg
      className="monitor-chart"
      viewBox="0 0 100 32"
      preserveAspectRatio="none"
      aria-hidden="true"
      data-point-count={values.length}
    >
      {segments.map((points, index) => (
        <polyline key={`${index}:${points}`} points={points} />
      ))}
    </svg>
  );
}

function ResourceValue({
  used,
  total,
  unavailable,
}: {
  used: number | null;
  total: number | null;
  unavailable: boolean;
}) {
  if (unavailable || used === null || total === null)
    return <span className="metric-unavailable">Unavailable</span>;
  return (
    <>
      {bytes(used)} <span className="metric-total">/ {bytes(total)}</span>
    </>
  );
}

export function LiveMonitoring({ hostId, hostSessionId, connected, visible }: Props) {
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
        const sample = await monitorApi.sample(hostId, hostSessionId);
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
    void poll();
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

  const visibleHistory = timeline.owner === owner ? timeline.samples : [];
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

  return (
    <section className="monitoring-section" aria-labelledby="live-monitoring-title">
      <div className="monitoring-heading">
        <div>
          <div className="eyebrow">SESSION METRICS</div>
          <h2 id="live-monitoring-title">Live monitoring</h2>
        </div>
        <span
          className={`monitor-freshness ${age > STALE_AFTER_MS ? 'monitor-freshness--stale' : ''}`}
        >
          {freshness}
        </span>
      </div>
      {error && (
        <p className="monitor-error" role="status">
          {error}
        </p>
      )}
      <div className="monitor-grid">
        <article className="monitor-card">
          <span className="metric-label">CPU</span>
          <strong>
            {cpuUnavailable
              ? 'Unavailable'
              : sample?.cpuUsagePercent === null || !sample
                ? 'Warming up'
                : formatPercent(sample.cpuUsagePercent)}
          </strong>
          <Sparkline values={visibleHistory.map((item) => item.cpuUsagePercent)} />
        </article>
        <article className="monitor-card">
          <span className="metric-label">MEMORY</span>
          <strong>
            <ResourceValue
              used={sample?.memoryUsedBytes ?? null}
              total={sample?.memoryTotalBytes ?? null}
              unavailable={memoryUnavailable}
            />
          </strong>
          <span>{formatPercent(memoryPercent)}</span>
          <Sparkline
            values={visibleHistory.map((item) =>
              percentage(item.memoryUsedBytes, item.memoryTotalBytes),
            )}
          />
        </article>
        <article className="monitor-card">
          <span className="metric-label">SWAP</span>
          <strong>
            {sample?.swapTotalBytes === 0 && !swapUnavailable ? (
              'Not configured'
            ) : (
              <ResourceValue
                used={sample?.swapUsedBytes ?? null}
                total={sample?.swapTotalBytes ?? null}
                unavailable={swapUnavailable}
              />
            )}
          </strong>
          {sample?.swapTotalBytes !== 0 && <span>{formatPercent(swapPercent)}</span>}
        </article>
        <article className="monitor-card">
          <span className="metric-label">ROOT DISK</span>
          <strong>
            <ResourceValue
              used={sample?.rootUsedBytes ?? null}
              total={sample?.rootTotalBytes ?? null}
              unavailable={diskUnavailable}
            />
          </strong>
          <span>{formatPercent(diskPercent)}</span>
        </article>
        <article className="monitor-card monitor-card--network">
          <span className="metric-label">NETWORK</span>
          <strong>
            {networkUnavailable
              ? 'Unavailable'
              : sample?.networkRxBytesPerSecond === null || !sample
                ? 'Warming up'
                : `↓ ${bytesPerSecond(sample.networkRxBytesPerSecond)}`}
          </strong>
          <span>
            {networkUnavailable
              ? ''
              : sample?.networkTxBytesPerSecond === null || !sample
                ? 'Waiting for the next sample'
                : `↑ ${bytesPerSecond(sample.networkTxBytesPerSecond)}`}
          </span>
          <Sparkline values={visibleHistory.map((item) => item.networkRxBytesPerSecond)} />
        </article>
      </div>
    </section>
  );
}
