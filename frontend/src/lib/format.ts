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

/**
 * A contributor's compute time to the second, in every unit it has: "45s",
 * "2m 13s", "5h 20m 13s", "3d 4h 5m 6s", "2y 17d 1h"; a unit that is zero is
 * left out. `duration` gives one unit to a decimal, which reads well for a
 * time left but blurs a total that runs to years across the whole fleet; and
 * the two largest units alone hid a list that the page refreshes every half
 * minute moving at all.
 */
export function computeTime(seconds: number | null): string {
  if (seconds === null || !isFinite(seconds) || seconds < 0) return '—';
  let s = Math.floor(seconds);
  if (s === 0) return '0s';
  const parts: string[] = [];
  for (const [size, unit] of [
    [365 * 86400, 'y'],
    [86400, 'd'],
    [3600, 'h'],
    [60, 'm'],
    [1, 's']
  ] as const) {
    const count = Math.floor(s / size);
    s -= count * size;
    if (count) parts.push(`${count.toLocaleString()}${unit}`);
  }
  return parts.join(' ');
}

/**
 * What a job page says of its tasks that hit the job's time limit
 * (`limitSeconds`): workers stop a task at the limit and hand it back (or the
 * server takes it back a minute later), so a job whose batch is too big for
 * it makes no progress, and the cure is the admin's -- a smaller batch, or a
 * longer limit for this job. `null` while none has.
 */
export function timeLimitNotice(declines: number, limitSeconds: number): string | null {
  if (!(declines > 0)) return null;
  const tasks = declines === 1 ? '1 task' : `${declines.toLocaleString()} tasks`;
  return `${tasks} hit this job's ${computeTime(limitSeconds)} time limit — lower the batch size, or raise the limit`;
}

/**
 * A count to its last digit, grouped: "1,234,567,890". Movegens run to
 * billions per contributor; rounded to "1.2B" the list stood still between
 * refreshes while every contributor's count was rising.
 */
export function exactCount(n: number | null): string {
  if (n === null || !isFinite(n) || n < 0) return '—';
  return Math.floor(n).toLocaleString();
}

/**
 * A job's pace in its own unit: "1,240 games/hour", "8 pairs/hour",
 * "2.5 racks/hour". Whole units from ten up and a decimal below, where a
 * rounded slow job read the same at twice the speed. "—" with no recent work.
 */
export function throughputText(t: { per_hour: number; unit: string } | null): string {
  if (!t || !isFinite(t.per_hour) || t.per_hour < 0) return '—';
  const n = t.per_hour >= 10 ? Math.round(t.per_hour).toLocaleString() : String(Math.round(t.per_hour * 10) / 10);
  return `${n} ${n === '1' ? t.unit : `${t.unit}s`}/hour`;
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
  opening_rack: 'Opening Rack Analysis',
  games: 'Games',
  game_pairs: 'Game Pairs',
  leave_generation: 'Leave Generation'
};

export function jobTypeLabel(type: string): string {
  return lookup(JOB_TYPE_LABELS, type);
}

const DERIVED_KIND_LABELS: Record<string, string> = {
  wmp: 'Wordmap',
  rit: 'Rack info table',
  wit: 'Word info table'
};

/** What a derived file is, from its role. */
export function derivedKind(role: string): string {
  return lookup(DERIVED_KIND_LABELS, role);
}

const TEST_LABELS: Record<string, string> = {
  running: 'running',
  paused: 'paused while the job is inactive',
  undecided: 'not decided: the job was completed before the test was',
  player1_better: 'decided: player 1 is better',
  player2_better: 'decided: player 2 is better',
  inconclusive: 'inconclusive: the job reached its cap first',
  off: 'not run: the job plays to its target'
};

export function testLabel(status: string): string {
  return lookup(TEST_LABELS, status);
}

/**
 * Where a games or pairs job's significance test stands, for its badge and label.
 * The test itself says `running` whenever its interval has not left an even
 * score behind, which read as work going on while the job was inactive and
 * nothing was being played: the job's status comes first. A completed job
 * shows the decision it was completed on, or `undecided` when it was
 * completed without one (an admin's force-complete). A job that runs no test
 * is `off` whatever its status.
 */
export function testState(
  jobStatus: string,
  games: { test: { status: string } | null; decided?: { status: string } }
): string {
  if (!games.test) return 'off';
  if (games.decided) return games.decided.status;
  if (jobStatus === 'inactive') return 'paused';
  if (jobStatus === 'completed') return 'undecided';
  return games.test.status;
}

/** A score per game as a percentage, to a tenth: 0.5312 is "53.1%". */
export function scorePct(score: number): string {
  return `${(score * 100).toFixed(1)}%`;
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
 * A comma-separated list of whole numbers as a form field holds one: blank is
 * `null` ("MAGPIE's default"), and anything that is not a list of whole
 * numbers is an error naming the part that is not.
 */
export function optionalIntList(text: string): { values: number[] | null } | { error: string } {
  const trimmed = text.trim();
  if (trimmed === '') return { values: null };
  const values: number[] = [];
  for (const part of trimmed.split(',').map((p) => p.trim())) {
    if (!/^\d+$/.test(part)) return { error: `"${part}" is not a whole number.` };
    values.push(Number(part));
  }
  return { values };
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
 * Why a form's selects, keyed by what they choose, cannot be sent: "Choose a
 * letter distribution and a board layout.", or null when every one holds an
 * id. An empty id sent reads as a malformed body to the server, whose answer
 * names no field.
 */
export function unchosenText(choices: Record<string, string>): string | null {
  const missing = Object.entries(choices)
    .filter(([, id]) => id === '')
    .map(([what]) => what);
  if (!missing.length) return null;
  const list =
    missing.length === 1
      ? missing[0]
      : `${missing.slice(0, -1).join(', ')} and ${missing[missing.length - 1]}`;
  return `Choose ${list}.`;
}

/**
 * Why a completed job finished, as a sentence: the job page said only
 * "completed", and a pairs job its test stopped read like one stopped at its
 * cap. From the completion record and, for a games or pairs job, the decision
 * and the test's bounds -- or, for one that runs no test, its target.
 */
export function completionText(stats: {
  job: { job_type: string };
  completion?: { forced: boolean; reason: string | null };
  games?: {
    unit: string;
    max_units: number;
    test: { confidence_pct: number } | null;
    decided?: { status: string; lower: number; upper: number; units: number };
  };
}): string {
  const completion = stats.completion;
  const games = stats.games;
  const units = (n: number, unit: string) => `${n.toLocaleString()} ${unit}${n === 1 ? '' : 's'}`;
  if (completion?.forced) {
    return games?.test && !games.decided
      ? 'an admin force-completed it before its test decided'
      : 'an admin force-completed it';
  }
  const decided = games?.decided;
  const test = games?.test;
  if (games && decided && test) {
    const interval = `player 1 scored ${scorePct(decided.lower)} to ${scorePct(decided.upper)} per game`;
    const at = `at ${test.confidence_pct}% confidence after ${units(decided.units, games.unit)}`;
    switch (decided.status) {
      case 'player1_better':
        return `its significance test found player 1 better ${at}: ${interval}`;
      case 'player2_better':
        return `its significance test found player 2 better ${at}: ${interval}`;
      case 'inconclusive':
        return `it reached its cap of ${units(games.max_units, games.unit)} before its significance test decided: ${interval}, at ${test.confidence_pct}% confidence`;
    }
  }
  // A job without a test has only its target to reach.
  if (games && completion?.reason === 'reached_target') {
    return `it played the ${units(games.max_units, games.unit)} it was set to`;
  }
  if (completion?.reason === 'last generation built') return 'its last generation was built';
  if (stats.job.job_type === 'opening_rack' && completion) return 'every rack was settled';
  return 'it was completed';
}

/** The most generations a leave job runs, and the highest target one may set (the server's bounds). */
export const MAX_LEAVE_GENERATIONS = 100;
export const MAX_TARGET_RACK_COUNT = 1_000_000;

/**
 * A leave job's per-generation rack targets, typed as MAGPIE's `leavegen`
 * takes them -- "100, 200, 500, 1000" -- one occurrence target per generation,
 * the list's length the number of generations. Spaces are ignored and a
 * trailing comma forgiven. Either the list or why it cannot be sent.
 */
export function parseTargetRackCounts(text: string): { targets: number[] } | { error: string } {
  // Commas or spaces between targets: MAGPIE's own `leavegen 100,200,500`
  // form, or a list typed with spaces. A thousands separator cannot be told
  // from a list separator, so "1,000" splits into 1 and 000 -- refused below,
  // and the preview (targetsText) shows any other misreading.
  const parts = text.trim().split(/\s*,\s*|\s+/);
  if (parts.length > 1 && parts[parts.length - 1] === '') parts.pop();
  if (parts.length === 1 && parts[0] === '') {
    return { error: 'List at least one generation\'s target, e.g. 100, 200, 500.' };
  }
  const targets: number[] = [];
  for (const [i, part] of parts.entries()) {
    if (!/^\d+$/.test(part)) {
      return { error: `"${part}" is not a whole number of occurrences.` };
    }
    if (i > 0 && /^0\d\d$/.test(part)) {
      return {
        error: `"${parts[i - 1]},${part}" reads as two targets: write ${parts[i - 1]}${part} without a thousands separator.`
      };
    }
    const target = Number(part);
    if (target < 1 || target > MAX_TARGET_RACK_COUNT) {
      return {
        error: `Every target must be between 1 and ${MAX_TARGET_RACK_COUNT.toLocaleString()}, not ${part}.`
      };
    }
    targets.push(target);
  }
  if (targets.length > MAX_LEAVE_GENERATIONS) {
    return { error: `At most ${MAX_LEAVE_GENERATIONS} generations, not ${targets.length}.` };
  }
  return { targets };
}

/**
 * A leave job's per-generation targets in generation order, e.g.
 * "100 → 200 → 1,000". Arrows rather than commas: the numbers carry
 * thousands separators, so a comma-separated list cannot be read.
 */
export function targetsText(targets: number[]): string {
  return targets.map((t) => t.toLocaleString()).join(' → ');
}

/**
 * Why a player config cannot be a leave job's player, or null if it can --
 * what job creation refuses, said before the submit. Leave values are measured
 * from static play ranked on equity (a score sort ignores the very leaves each
 * generation feeds back), and a rack info table caches the leaves every
 * generation replaces.
 */
export function leavePlayerConflict(p: {
  name: string;
  num_plies: number;
  sort_strategy: string;
  use_rit: boolean;
  /** Absent on a config read before the setting existed: it solves nothing. */
  endgame_plies?: number;
}): string | null {
  const problems: string[] = [];
  if (p.num_plies > 0) problems.push(`simulates ${p.num_plies} ${p.num_plies === 1 ? 'ply' : 'plies'}`);
  if (p.sort_strategy !== 'equity') problems.push(`sorts on ${p.sort_strategy}`);
  if (p.use_rit) problems.push('asks for a rack info table');
  // A leave game ends before the bag is small enough for either solver.
  if ((p.endgame_plies ?? 0) > 0) problems.push('solves endgames');
  if (!problems.length) return null;
  const list =
    problems.length === 1 ? problems[0] : `${problems.slice(0, -1).join(', ')} and ${problems[problems.length - 1]}`;
  return `${p.name} ${list}; leave generation plays statically on equity, without a rack info table or endgame solving.`;
}

/** A job's title: the name it was given, or its type for one given none. */
export function jobTitle(job: { name?: string | null; job_type: string }): string {
  const name = job.name?.trim();
  return name ? name : jobTypeLabel(job.job_type);
}

/**
 * What the admin job page's Export card says about the job's newest export, and
 * what its button offers.
 *
 * A job can be exported at any time. A **snapshot** -- read while the job was
 * still taking results -- is downloadable but is not the job's corpus, and is
 * labelled with its time; a completed job whose newest export is one is offered
 * its final export. A leave job's snapshot is as of its last merge, which runs
 * every half hour while it runs, and says so.
 *
 * A completed opening-rack job's final export also becomes a snapshot when its
 * consensus settings change, even to ones that leave it completed: it was read
 * after the job completed, under the old settings. The export does not record
 * which happened, so that job's label names both.
 */
export function exportSummary(
  jobExport: { state: string; is_final: boolean; snapshot_at: string | null } | null,
  job: { status: string; job_type: string }
): { label: string | null; note: string | null; button: string } {
  const completed = job.status === 'completed';
  const built = jobExport !== null && jobExport.state !== 'running' && jobExport.state !== 'failed';
  const snapshot = built && !jobExport.is_final;
  let label: string | null = null;
  if (snapshot) {
    const at = datetime(jobExport.snapshot_at);
    if (!completed) label = `Snapshot as of ${at} — job still running`;
    else if (job.job_type === 'opening_rack')
      label = `Snapshot as of ${at} — not the job's final results: read before it completed, or before its consensus settings last changed`;
    else label = `Snapshot as of ${at}, read before the job completed — not its final results`;
  } else if (built) {
    label = 'Final results';
  }
  const note =
    snapshot && job.job_type === 'leave_generation'
      ? 'Rack totals as of the last merge, which runs every half hour while the job runs.'
      : null;
  let button: string;
  if (!completed) button = jobExport === null ? 'Export a snapshot' : 'Export a new snapshot';
  else if (jobExport === null) button = 'Export results';
  // A failed build is retried as what it was for: the completed job's corpus.
  else if (snapshot || jobExport.state === 'failed') button = 'Build the final export';
  else button = 'Export again';
  return { label, note, button };
}
