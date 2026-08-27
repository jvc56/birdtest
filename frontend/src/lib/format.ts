/** Shared display helpers, so a worker or a duration reads the same everywhere. */

export function workerLabel(worker: {
  username?: string | null;
  anon_uuid?: string | null;
}): string {
  if (worker.username) return worker.username;
  if (worker.anon_uuid) return `Anonymous · ${worker.anon_uuid.slice(0, 8)}`;
  return 'Unknown';
}

export function duration(seconds: number | null): string {
  if (seconds === null || !isFinite(seconds)) return '—';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86400) return `${(seconds / 3600).toFixed(1)}h`;
  return `${(seconds / 86400).toFixed(1)}d`;
}

export function datetime(value: string | null): string {
  if (!value) return '—';
  return new Date(value).toLocaleString();
}

export function jobTypeLabel(type: string): string {
  return (
    {
      opening_rack_analysis: 'Opening rack analysis',
      games: 'Games',
      game_pairs: 'Game pairs',
      leave_generation: 'Leave generation'
    }[type] ?? type
  );
}

export function sprtLabel(status: string): string {
  return (
    {
      running: 'running',
      passed: 'passed (H1 accepted)',
      failed: 'failed (H0 accepted)',
      terminated_at_max: 'terminated at max games'
    }[status] ?? status
  );
}
