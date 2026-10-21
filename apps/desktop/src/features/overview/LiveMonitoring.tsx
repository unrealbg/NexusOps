import { useMemo } from 'react';
import { bytes, bytesPerSecond, percentage } from './format';

import { useHostMonitoring, formatPercent, type MonitoringProps } from './useHostMonitoring';
export { MONITOR_INTERVAL_MS, MONITOR_HISTORY_LIMIT } from './useHostMonitoring';
const STALE_AFTER_MS = 15_000;

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

export function LiveMonitoring(props: MonitoringProps) {
  const { visibleHistory, error, sample, age, freshness, cpuUnavailable, networkUnavailable,
    memoryUnavailable, swapUnavailable, diskUnavailable, memoryPercent, swapPercent, diskPercent } = useHostMonitoring(props);

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
