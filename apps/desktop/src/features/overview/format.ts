export function bytes(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return 'Unavailable';
  if (value === 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toLocaleString(undefined, { maximumFractionDigits: index > 0 ? 1 : 0 })} ${units[index]}`;
}

export function uptime(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return 'Unavailable';
  const days = Math.floor(value / 86400);
  const hours = Math.floor((value % 86400) / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  return days ? `${days}d ${hours}h` : hours ? `${hours}h ${minutes}m` : `${minutes}m`;
}

export function percentage(used: number | null, total: number | null): number | null {
  return used !== null &&
    total !== null &&
    Number.isFinite(used) &&
    Number.isFinite(total) &&
    used >= 0 &&
    total > 0
    ? Math.max(0, Math.min(100, (used / total) * 100))
    : null;
}

export function observedTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? 'Time unavailable'
    : date.toLocaleString(undefined, {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      });
}
