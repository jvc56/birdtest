/** Shared display helpers, so a worker or a duration reads the same everywhere. */

export function workerLabel(worker: {
  username?: string | null;
  anon_id?: string | null;
}): string {
  if (worker.username) return worker.username;
  // A pseudonym derived from the anonymous worker's UUID, never the UUID
  // itself, which is the worker's credential.
  if (worker.anon_id) return `Anonymous · ${worker.anon_id.slice(0, 8)}`;
  return 'Unknown';
}

export function duration(seconds: number | null): string {
  if (seconds === null || !isFinite(seconds)) return '—';
  // Each unit is chosen by the value as it will be displayed, after rounding,
  // so 59.6 s reads "1m" rather than "60s", and 3599 s "1.0h" rather than "60m".
  if (Math.round(seconds) < 60) return `${Math.round(seconds)}s`;
  if (Math.round(seconds / 60) < 60) return `${Math.round(seconds / 60)}m`;
  if (Math.round(seconds / 360) < 240) return `${(seconds / 3600).toFixed(1)}h`;
  return `${(seconds / 86400).toFixed(1)}d`;
}

export function datetime(value: string | null): string {
  if (!value) return '—';
  return new Date(value).toLocaleString();
}

/**
 * Look a key up in a label table, falling back to the key itself. An own-
 * property check rather than `table[key] ?? key`, which would hand back
 * `Object.prototype.toString` for the key "toString".
 */
function lookup(table: Record<string, string>, key: string): string {
  return Object.prototype.hasOwnProperty.call(table, key) ? table[key] : key;
}

const JOB_TYPE_LABELS: Record<string, string> = {
  opening_rack: 'Opening rack analysis',
  games: 'Games',
  game_pairs: 'Game pairs',
  leave_generation: 'Leave generation'
};

export function jobTypeLabel(type: string): string {
  return lookup(JOB_TYPE_LABELS, type);
}

const SPRT_LABELS: Record<string, string> = {
  running: 'running',
  passed: 'passed (H1 accepted)',
  failed: 'failed (H0 accepted)',
  terminated_at_max: 'terminated at max games'
};

export function sprtLabel(status: string): string {
  return lookup(SPRT_LABELS, status);
}

/**
 * An optional number field's value as the API wants it: blank is `null`
 * ("MAGPIE's default"). Svelte binds a cleared number box as `null`, not `''`,
 * and `Number(null)` is 0 -- which was written into a config that cannot be
 * edited.
 */
export function optionalNumber(value: number | string | null | undefined): number | null {
  if (value === '' || value === null || value === undefined) return null;
  const number = Number(value);
  return Number.isNaN(number) ? null : number;
}

/**
 * The fields of a request body left blank: `null`, or a number box's `NaN`.
 * Sent, the server's answer to one names the whole body ("data did not match
 * any variant"), not the field.
 */
export function blankFields(body: Record<string, unknown>): string[] {
  return Object.entries(body)
    .filter(([, value]) => value === null || (typeof value === 'number' && Number.isNaN(value)))
    .map(([field]) => field);
}
