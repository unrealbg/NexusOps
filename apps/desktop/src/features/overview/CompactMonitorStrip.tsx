import { bytesPerSecond } from './format';
import { formatPercent, useHostMonitoring, type MonitoringProps } from './useHostMonitoring';

export function CompactMonitorStrip(props: MonitoringProps) {
  const monitor = useHostMonitoring(props);
  if (!props.visible) return null;
  const { sample } = monitor;
  const cpu = monitor.cpuUnavailable ? 'Unavailable' : sample?.cpuUsagePercent == null ? 'Warming up' : formatPercent(sample.cpuUsagePercent);
  const rate = (value: number | null | undefined) => monitor.networkUnavailable ? 'Unavailable' : value == null ? 'Warming up' : bytesPerSecond(value);
  return (
    <section className="compact-monitor" aria-label="SSH workspace monitoring">
      <span className="compact-monitor-freshness" role="status">{monitor.freshness}</span>
      <span>CPU <strong>{cpu}</strong></span>
      <span>RAM <strong>{monitor.memoryUnavailable ? 'Unavailable' : formatPercent(monitor.memoryPercent)}</strong></span>
      <span>Root <strong>{monitor.diskUnavailable ? 'Unavailable' : formatPercent(monitor.diskPercent)}</strong></span>
      <span>RX <strong>{rate(sample?.networkRxBytesPerSecond)}</strong></span>
      <span>TX <strong>{rate(sample?.networkTxBytesPerSecond)}</strong></span>
      {monitor.error && <span className="monitor-error" role="status">{monitor.error}</span>}
    </section>
  );
}
