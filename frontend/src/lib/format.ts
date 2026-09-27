/** Shared display helpers, so a worker or a duration reads the same everywhere. */

export function workerLabel(worker: {
  username?: string | null;
  anon_id?: string | null;
}): string {
  if (worker.username) return worker.username;
  // A pseudonym derived from the anonymous worker's UUID, never the UUID
  // itself, which is the worker's credential.
  // The pseudonym whole, sixteen hex characters: it is what `?worker=` takes,
  // and eight were shared by several contributors at a few hundred thousand.
  // Anything else -- never what the server sends -- only by a prefix, so a
  // credential passed here by mistake does not appear.
  if (worker.anon_id) {
    const whole = /^[0-9a-f]{16}$/.test(worker.anon_id);
    return `Anonymous · ${whole ? worker.anon_id : worker.anon_id.slice(0, 8)}`;
  }
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
  paused: 'paused while the job is inactive',
  undecided: 'not decided: the job was completed before the test was',
  passed: 'passed (H1 accepted)',
  failed: 'failed (H0 accepted)',
  terminated_at_max: 'stopped at its cap'
};

export function sprtLabel(status: string): string {
  return lookup(SPRT_LABELS, status);
}

/**
 * Where a games or pairs job's test stands, for its badge and label. The test
 * itself says `running` whenever it has not crossed a bound, which read as
 * work going on while the job was inactive and nothing was being played:
 * the job's status comes first. A completed job shows the decision it was
 * completed on, or `undecided` when it was completed without one (an admin's
 * force-complete).
 */
export function sprtState(
  jobStatus: string,
  games: { sprt: { status: string }; decided?: { status: string } }
): string {
  if (games.decided) return games.decided.status;
  if (jobStatus === 'inactive') return 'paused';
  if (jobStatus === 'completed') return 'undecided';
  return games.sprt.status;
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

/**
 * Why a completed job finished, as a sentence: the job page said only
 * "completed", and a pairs job its test stopped read like one stopped at its
 * cap. From the completion record and, for a games or pairs job, the decision
 * and the test's bounds.
 */
export function completionText(stats: {
  job: { job_type: string };
  completion?: { forced: boolean; reason: string | null };
  games?: {
    unit: string;
    max_units: number;
    sprt: { lower_bound: number; upper_bound: number };
    decided?: { status: string; llr: number; units: number };
  };
}): string {
  const completion = stats.completion;
  const games = stats.games;
  const units = (n: number, unit: string) => `${n.toLocaleString()} ${unit}${n === 1 ? '' : 's'}`;
  if (completion?.forced) {
    return games && !games.decided
      ? 'an admin force-completed it before its test decided'
      : 'an admin force-completed it';
  }
  const decided = games?.decided;
  if (games && decided) {
    const llr = decided.llr.toFixed(3);
    switch (decided.status) {
      case 'passed':
        return `the SPRT passed (H1 accepted) after ${units(decided.units, games.unit)}: LLR ${llr} reached the upper bound ${games.sprt.upper_bound.toFixed(2)}`;
      case 'failed':
        return `the SPRT failed (H0 accepted) after ${units(decided.units, games.unit)}: LLR ${llr} reached the lower bound ${games.sprt.lower_bound.toFixed(2)}`;
      case 'terminated_at_max':
        return `it reached its cap of ${units(games.max_units, games.unit)} before the SPRT decided (LLR ${llr}, bounds [${games.sprt.lower_bound.toFixed(2)}, ${games.sprt.upper_bound.toFixed(2)}])`;
    }
  }
  if (completion?.reason === 'last generation built') return 'its last generation was built';
  if (stats.job.job_type === 'opening_rack' && completion) return 'every rack was analysed';
  return 'it was completed';
}

/** A job's title: the name it was given, or its type for one given none. */
export function jobTitle(job: { name?: string | null; job_type: string }): string {
  const name = job.name?.trim();
  return name ? name : jobTypeLabel(job.job_type);
}
