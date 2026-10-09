/**
 * Typed fetch wrappers for every API endpoint.
 *
 * Two things are handled centrally here so no call site has to remember them:
 * session cookies (`credentials: 'include'`) and the CSRF double-submit header,
 * which every state-mutating request needs and no read does.
 */

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly fields: Record<string, string> = {},
    /** Seconds to wait, from `Retry-After`, when the server gave one. */
    readonly retryAfter: number | null = null
  ) {
    super(message);
  }
}

/** `Retry-After` in seconds; its HTTP-date form is not one the server sends. */
function retryAfterSeconds(header: string | null): number | null {
  const seconds = header === null ? NaN : Number(header);
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : null;
}

/**
 * What a form shows for a failed request: the message, and which fields the
 * server said were wrong and why. The message alone ("job settings are
 * invalid") tells the admin nothing to change.
 */
export function errorText(e: unknown): string {
  const message = e instanceof Error ? e.message : String(e);
  const fields = e instanceof ApiError ? Object.entries(e.fields) : [];
  return fields.length
    ? `${message}: ${fields.map(([field, why]) => `${field} ${why}`).join('; ')}`
    : message;
}

function csrfToken(): string {
  const match = document.cookie.match(/(?:^|;\s*)birdtest_csrf=([^;]+)/);
  return match ? decodeURIComponent(match[1]) : '';
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (method !== 'GET') headers['x-csrf-token'] = csrfToken();

  const response = await fetch(path, {
    method,
    headers,
    credentials: 'include',
    body: body === undefined ? undefined : JSON.stringify(body)
  });

  if (response.status === 204) return undefined as T;

  const text = await response.text();
  // A body that is not JSON — a proxy's HTML error page, a truncated response —
  // must still surface as an ApiError, never as a bare SyntaxError that the
  // call sites' error handling does not expect.
  let payload: any;
  let parsed = true;
  try {
    payload = text ? JSON.parse(text) : undefined;
  } catch {
    payload = undefined;
    parsed = false;
  }

  if (response.ok && !parsed) {
    throw new ApiError(response.status, 'invalid_response', 'The server sent an unreadable response.');
  }

  if (!response.ok) {
    const fields: Record<string, string> = {};
    for (const entry of payload?.fields ?? []) fields[entry.field] = entry.message;
    throw new ApiError(
      response.status,
      payload?.code ?? 'error',
      // Over HTTP/2 -- the load balancer's -- `statusText` is always empty,
      // so an answer with no JSON (the ALB's own 502/503 during a deploy)
      // produced an error with no message, and pages showed nothing at all.
      payload?.message ?? (response.statusText || `The server answered ${response.status}.`),
      fields,
      retryAfterSeconds(response.headers.get('retry-after'))
    );
  }
  return payload as T;
}

const get = <T>(path: string) => request<T>('GET', path);
const post = <T>(path: string, body?: unknown) => request<T>('POST', path, body ?? {});
const patch = <T>(path: string, body: unknown) => request<T>('PATCH', path, body);
const put = <T>(path: string, body: unknown) => request<T>('PUT', path, body);
const del = <T>(path: string) => request<T>('DELETE', path);

/**
 * A query string from parameters, leaving out any that are `undefined`: a
 * first page has no cursor, and `String(undefined)` sent `cursor=undefined`,
 * which the server read as "from the start" only because it ignores a cursor
 * it cannot decode.
 */
export function query(params: Record<string, string | number | undefined>): string {
  return new URLSearchParams(
    Object.entries(params).flatMap(([k, v]) => (v === undefined ? [] : [[k, String(v)]]))
  ).toString();
}

// --- Shared shapes ---------------------------------------------------------

export type JobType = 'opening_rack' | 'games' | 'game_pairs' | 'leave_generation';
export type JobStatus = 'active' | 'inactive' | 'completed';

export interface Page<T> {
  items: T[];
  total: number;
  page: number;
  per_page: number;
}

/** A position a games or pairs job captured, with its ranked moves. */
export interface SavedPosition {
  task_id: string;
  game_index: number;
  turn_number: number;
  rack: string;
  /** CGP. */
  position: string | null;
  previous_move: string | null;
  previous_move_score: number | null;
  /**
   * The move played from this position, and its score: the one chosen, which
   * need not be the top of `moves` (a simmer's pick, or a solver's).
   */
  played_move: string | null;
  played_move_score: number | null;
  num_moves: number;
  /** How the move played here was chosen. */
  analysis: PositionAnalysis;
  submitted_at: string;
  /**
   * A game-pairs job's only: the same turn of the pair's other game (the
   * game index with its low bit flipped), or null when that game has no
   * position at that turn. Absent for a games job.
   */
  partner?: SavedPosition | null;
  moves: {
    rank: number;
    move: string;
    score: number;
    equity: number;
    /** How often a simulation played the move out; 0 or null when nothing simulated it. */
    iterations: number | null;
    /** A simulation's or a pre-endgame solve's; null for static and endgame. */
    win_percentage: number | null;
    /** A solve's projected final spread for the mover, in points; null unless solved. */
    mean_spread: number | null;
    /** The endgame depth a solve ranked the move at; null unless solved. */
    fidelity_plies: number | null;
    /** A simulation's first two plies, in order; empty for a move nothing simulated. */
    plies: PlyStats[];
  }[];
  /**
   * What the simmer inferred of the opponent's leave from their previous
   * move, before it simmed: null when it did not infer (a static or solved
   * position, a game's first turn, a pass before it, or a player that does not
   * infer).
   */
  inference: Inference | null;
}

/** One ply of a move's simulation: ply 0 is the reply to it (shown as P1). */
export interface PlyStats {
  ply: number;
  bingo_percentage: number;
  average_score: number;
}

/**
 * An inference of the opponent's leave: how many distinct leaves it found,
 * how many it drew in all, their mean equity, and the most drawn of them (at
 * most ten), most drawn first.
 */
export interface Inference {
  num_leaves: number;
  total_draws: number;
  average_equity: number;
  leaves: { leave: string; draws: number; equity: number }[];
}

/**
 * One move of an opening rack's lookup: `analysis` numbers the rack's analyses
 * from 1 (a job seeking a consensus analyses a rack more than once).
 */
export interface RackLookupRow {
  analysis: number;
  rank: number;
  move: string;
  score: number;
  equity: number;
  /** How often the simulation played the move out; 0 or null when nothing simulated it. */
  iterations: number | null;
  /** A simulation's; null for a static analysis. */
  win_percentage: number | null;
  plies: PlyStats[];
}

/** Static equity, a simulation, a pre-endgame solve or an endgame solve. */
export type PositionAnalysis = 'static' | 'sim' | 'peg' | 'endgame';

/** What the contributor list can be ranked by; the server's default is movegens. */
export type ContributorSort = 'movegens' | 'compute' | 'tasks';

/** One row of the contributor list (`/api/workers`). */
export interface Contributor {
  user_id: string | null;
  /** An anonymous worker's public pseudonym; its UUID is never published. */
  anon_id: string | null;
  /** The anonymous worker's UUID, its credential: the admin list only. */
  anon_uuid?: string;
  username: string | null;
  /** Every accepted claim, held from claim to submission: the compute MAGPIE does not report. */
  compute_seconds: number;
  /** Every move generation MAGPIE reported for its accepted claims: the work done. */
  movegens: number;
  tasks_completed: number;
  /** The last task finished. */
  last_seen_at: string | null;
}

/** A layout square, by what it multiplies (`#` in MAGPIE's layout is a brick). */
export type BoardSquare =
  | 'normal'
  | 'double_letter'
  | 'double_word'
  | 'triple_letter'
  | 'triple_word'
  | 'quadruple_letter'
  | 'quadruple_word'
  | 'brick';

/** What a job's positions are drawn on: its board layout and what its tiles score. */
export interface BoardData {
  /** The square the first play covers, `[row, column]` from zero. */
  start: [number, number];
  /** Top row first. */
  squares: BoardSquare[][];
  /** In machine-letter order; the blank's own row is `?`. */
  letters: { letter: string; blank: string; score: number }[];
}

/**
 * A page by cursor rather than by offset, for a job's results and its captured
 * positions: the corpus runs to millions of rows, where `OFFSET` produces every
 * row before the page asked for. Pass `next_cursor` back as `cursor` for the
 * next page; its absence is the end. These two are the only endpoints that page
 * this way.
 */
export interface CursorPage<T> {
  items: T[];
  /**
   * -1 for a results or positions page (an exact count costs more than it is
   * worth to the caller); a rack lookup's count of the moves it returns.
   */
  total: number;
  per_page: number;
  next_cursor?: string;
}

export interface JobListItem {
  id: string;
  /** What the admin called it; empty for a job created without one. */
  name: string;
  job_type: JobType;
  status: JobStatus;
  /** The job's share of claims (not of worker time: PLAN's KL-88): above 0 exactly when the job is active, so 0% is what inactive means and a completed job holds 0. */
  allocation: number;
  created_at: string;
  tasks_total: number;
  tasks_completed: number;
  units_completed: number | null;
  max_units: number | null;
  /** Workers are declining this job and none is completing it — usually data nobody has. */
  stalled: boolean;
}

/**
 * A job's own row, as the admin actions (create, set allocations, complete)
 * return it -- not the list's summary, which adds counts.
 */
export interface JobRow {
  id: string;
  name: string;
  job_type: JobType;
  status: JobStatus;
  allocation: number;
  variant: string;
  created_at: string;
}

/**
 * A games or pairs job's significance test: a confidence interval for player 1's
 * score per game (1 a win, ½ a draw) that stays valid however often it is
 * checked. The job stops once it excludes an even score -- one player is
 * better -- or at its cap, inconclusive.
 */
export interface TestResult {
  /** Player 1's score per game, ½ before anything is played. */
  mean: number;
  lower: number;
  upper: number;
  confidence_pct: number;
  status: 'running' | 'player1_better' | 'player2_better' | 'inconclusive';
}

export interface GameStats {
  unit: 'game' | 'pair';
  /** Per-game counts over every game played, for both job types. */
  wins: number;
  losses: number;
  draws: number;
  /** Games for a `games` job, pairs for a `game_pairs` job. */
  units_completed: number;
  /**
   * Each player's average score per game and player 1's average spread, over
   * every game played (batches weighted by their games); null before any.
   */
  p1_score_mean: number | null;
  p2_score_mean: number | null;
  spread_mean: number | null;
  /**
   * Game pairs only: the five pair outcomes the test is computed from, indexed
   * by player 1's half-point score across the pair (0 = lost both, 4 = won
   * both). Every completed pair is in here, including the ones that played
   * identically — they are 1-1 ties in bucket 2, and they are what makes a
   * paired run lower-variance than an unpaired one.
   */
  pentanomial?: [number, number, number, number, number];
  /** Game pairs only: how many pairs diverged. A diagnostic, not the sample. */
  divergent_pairs?: number;
  /**
   * Game pairs only: the match score over the games of the pairs that
   * diverged, where the two configs played differently. A diagnostic beside
   * the full score.
   */
  divergent?: {
    wins: number;
    losses: number;
    draws: number;
    p1_score_mean: number | null;
    p2_score_mean: number | null;
    spread_mean: number | null;
  };
  min_units: number;
  max_units: number;
  /**
   * The significance test over every accepted result, recomputed on each read; null
   * for a job that runs none, which plays `max_units` and stops.
   */
  test: TestResult | null;
  /**
   * What a completed job stopped on, when the finish check completed it, with
   * player 1's interval then. Results in flight at that moment still land, so
   * `test` can move afterwards; this is the decision that stands.
   */
  decided?: { status: TestResult['status']; lower: number; upper: number; units: number };
}

/**
 * How a job was completed. `forced` is an admin's force-complete; otherwise
 * `reason` is the server's: the significance test's verdict (`player1_better`,
 * `player2_better`, `inconclusive`), `reached_target` for a games or pairs job without a
 * test, `last generation built`, or none for an opening-rack job whose racks
 * were all settled.
 */
export interface Completion {
  at: string;
  forced: boolean;
  reason: string | null;
}

export interface JobStats {
  job: {
    id: string;
    /** What the admin called it; empty for a job created without one. */
    name: string;
    job_type: JobType;
    status: JobStatus;
    allocation: number;
      min_magpie_version: string;
    created_at: string;
    created_by: string | null;
    /** The lexicons in play. A games job comparing two reads "CSW21 vs NWL23". */
    lexicon: string | null;
    variant: string | null;
    /** Tasks a worker stopped at the time limit and handed back. */
    time_limit_declines: number;
    /**
     * Why the server switched the job off, while it is off: three of its tasks
     * in a row hit the time limit. Null for a job an admin switched off.
     */
    set_aside_reason: string | null;
  };
  tasks_total: number;
  tasks_completed: number;
  tasks_available: number;
  tasks_claimed: number;
  games?: GameStats;
  opening_racks?: {
    racks_analyzed: number;
    /**
     * Racks needing no more analysis -- the job is done once all are -- and
     * those of them settled at their most analyses without a consensus. For a
     * job wanting one analysis per rack, `racks_settled` is `racks_analyzed`.
     */
    racks_settled: number;
    racks_without_consensus: number;
    /** Size of the rack space — the denominator for progress. */
    racks_total: number;
  };
  leave_generation?: {
    current_generation: number;
    /** Generations whose KLV is built; `current_generation` stops at the last one. */
    generations_closed: number;
    generation_count: number;
    /** The in-progress generation's occurrence target, from `target_rack_counts` (every generation's, in order). */
    target_rack_count: number;
    target_rack_counts: number[];
    /** Live: accepted tasks of the in-progress generation, and the games they played. */
    tasks_completed: number;
    games_played: number;
    /** As of `progress_as_of`: accepted results are merged into the rack totals in batches. */
    racks_at_target: number;
    racks_total: number;
    min_rack: string | null;
    min_rack_count: number | null;
    progress_as_of: string | null;
  };
  workers: {
    user_id: string | null;
    /** An anonymous worker's public pseudonym; its UUID is never published. */
    anon_id: string | null;
    username: string | null;
    tasks_completed: number;
    /** Those claims held from claim to submission. */
    compute_seconds: number;
  }[];
  /** Contributors beyond the ones listed; the list is capped. */
  other_workers: number;
  eta_seconds: number | null;
  /** How a completed job came to be completed; absent while it is not. */
  completion?: Completion;
}

export interface PlayerConfig {
  id: string;
  name: string;
  recorder_type: string;
  sort_strategy: string;
  /** The files this player pins, as input_data rows rather than names. */
  kwg_id: string;
  klv_id: string;
  /** Null for a static player, which never loads a win% model. */
  winpct_id: string | null;
  /** Set when this config was cloned onto newer data; a clone starts unrated. */
  cloned_from_id: string | null;
  /**
   * Every setting a task states is stated here: the server fills MAGPIE's
   * defaults in at creation. The nullable ones are simulation settings, null
   * for a static player (num_plies 0) and set for every simmer.
   */
  max_iterations: number | null;
  num_plies: number;
  num_plies_recorded: number;
  num_plays: number;
  num_plays_recorded: number;
  stopping_pct: number | null;
  use_inference: boolean | null;
  time_limit_secs: number | null;
  use_wordmap: boolean;
  use_rit: boolean;
  use_wit: boolean;
  min_play_iterations: number | null;
  threshold: string | null;
  sampling_rule: string | null;
  inference_margin: number | null;
  utility_w_winpct: number | null;
  utility_w_spread: number | null;
  utility_spread_scale: number | null;
  movegen_margin: number;
  /**
   * Endgame and pre-endgame solving, for games and game-pairs jobs.
   * endgame_plies 0 solves nothing (and so no pre-endgame either); the PEG
   * settings are null unless peg_max_bag is above 0, and the nested ones
   * unless peg_nested is set.
   */
  endgame_plies: number;
  peg_max_bag: number;
  peg_stage_top_k: number[] | null;
  peg_scenario_stride: number | null;
  peg_opp_model: 'rational' | 'pessimistic' | null;
  peg_nested: boolean | null;
  peg_nested_cand_caps: number[] | null;
  peg_nested_max_depth: number | null;
  peg_nested_strides: number[] | null;
  created_at: string;
}

/** One input data file birdtest knows about, identified by content. */
export interface InputData {
  id: string;
  path: string;
  role: 'kwg' | 'klv' | 'winpct' | 'letterdist' | 'layout';
  name: string;
  sha256: string;
  bytes: number;
  /** The versioned tarball this content was FIRST seen in. */
  tarball_date: string;
  imported_at: string;
  /** Jobs and player configs pinning this row; a delete is refused while > 0. */
  references: number;
}

export interface ImportDetail {
  id: string;
  tarball_date: string;
  commit_sha: string;
  tarball_sha256: string | null;
  /** `nothing_new`: staged, but every file was already known. */
  state: 'running' | 'staged' | 'nothing_new' | 'confirmed' | 'cancelled' | 'failed';
  progress_bytes: number;
  progress_entries: number;
  error: string | null;
  requested_at: string;
  confirmed_at: string | null;
  files: {
    path: string;
    role: string;
    name: string;
    sha256: string;
    bytes: number;
    /** `collision` is a known path with different bytes — worth a second look. */
    disposition: 'new' | 'known' | 'collision';
  }[];
}

export interface DataGap {
  role: string;
  name: string;
  expected: string;
  workers: number;
  declines: number;
  last_reported_at: string;
}

export interface FleetVersion {
  magpie_version: string | null;
  workers: number;
  claims: number;
}

/** The settings an admin changes at run time (`/admin/settings`). */
export interface Settings {
  /**
   * The longest a task may run, in seconds (60 to 86,400). Every claim is
   * given the limit as it stands then; a worker stops a task at it and hands
   * it back.
   */
  max_task_seconds: number;
  /** Who changed them last; null until anyone has. */
  updated_by: string | null;
  updated_at: string;
}

/** One run of scripts/backup.sh, as recorded in the `backups` table. */
export interface BackupRun {
  id: string;
  kind: string;
  /** S3 key prefix, or a snapshot identifier. */
  location: string | null;
  started_at: string;
  finished_at: string;
  duration_seconds: number;
  dump_bytes: number | null;
  sha256: string | null;
  ok: boolean;
  total_rows: number;
}

export interface BackupStatus {
  last_success_at: string | null;
  last_success_age_seconds: number | null;
  /** No successful backup inside the alarm window — including never having had one. */
  stale: boolean;
  recent: BackupRun[];
}

/** One wordmap, rack info table or word info table a job needs, and where its build stands. */
export interface JobDerivedFile {
  /** `wmp`, `rit` or `wit`. */
  role: string;
  name: string;
  /** `pending` | `building` | `built` | `failed`. */
  state: string;
  error: string | null;
  attempts: number;
}

/**
 * A wordmap, rack info table or word info table the server builds a reference
 * copy of.
 *
 * None of them is shipped — 179 MB, 1.9 GB and 122 MB for CSW24 — so what
 * travels to a worker is the SHA-256 the server's own pinned MAGPIE got from the same
 * inputs. A job that needs one is not dispatched until it is `built`, which is
 * why this page exists: an active job doing nothing usually has a row here.
 */
export interface DerivedData {
  /** `wmp`, `rit` or `wit`. */
  role: string;
  /** What the worker loads it as: a lexicon, or `<lexicon>.<leaves>`. */
  name: string;
  /** The builder that produced the hash, e.g. `wmp-1`. */
  builder: string;
  /** Its files, as paths and the tarballs they came from. */
  made_from: string;
  /** Whether a builder of this server's MAGPIE takes it. */
  buildable: boolean;
  /** The files it is built from: what tells two rows of one name apart. */
  kwg_id: string;
  klv_id: string | null;
  letterdist_id: string;
  /** `pending` | `building` | `built` | `failed`. */
  state: string;
  sha256: string | null;
  bytes: number | null;
  build_target: string | null;
  error: string | null;
  attempts: number;
  requested_at: string;
  built_at: string | null;
}

/** A ban in force; `id` is what lifting it takes. */
export interface WorkerBan {
  id: string;
  user_id: string | null;
  username: string | null;
  anon_uuid: string | null;
  reason: string | null;
  created_at: string;
}

/** A completed job's results as one gzipped NDJSON object; see PLAN.md, "Exports". */
export interface JobExport {
  id: string;
  /** `expired`: built, but older than the artifact store keeps exports. */
  state: 'running' | 'ready' | 'expired' | 'failed';
  bytes: number | null;
  sha256: string | null;
  row_count: number | null;
  /**
   * A games or game-pairs job that captured positions has a second object
   * holding them, each with its ranked moves. Null for every other export.
   */
  positions_bytes: number | null;
  positions_sha256: string | null;
  positions_row_count: number | null;
  /**
   * True for a completed job's final corpus; false for a snapshot read while
   * the job was still taking results, for a completed job's export whose
   * consensus settings were changed after it was built, and for an export not
   * yet built.
   */
  is_final: boolean;
  /** When the snapshot it was read in was taken; null until built. */
  snapshot_at: string | null;
  error: string | null;
  requested_at: string;
  completed_at: string | null;
  /** Present once ready: a presigned URL, valid for an hour, that fetches the object directly. */
  download_url?: string;
  /** The same for the captured positions, when the export has them. */
  positions_download_url?: string;
}

/** Per generation, what rebuilding a leave job's KLV from the database found. */
export interface ArtifactRebuild {
  generation: number;
  artifact_key: string;
  stored_sha256: string;
  rebuilt_sha256: string;
  /**
   * Whether the stored artifact was written by the builder this rebuild used.
   * When it was not, `matches` says nothing: MAGPIE built these, so an upgrade
   * can legitimately change the bytes.
   */
  same_builder: boolean;
  stored_builder: string;
  rebuilt_builder: string;
  matches: boolean;
  object_present: boolean;
  rewritten: boolean;
  /** The hash workers are sent and verify against. */
  served_sha256: string;
  /** The object's hash as the check found it; null when it was missing. */
  object_sha256: string | null;
  /**
   * Whether the object held the first build, this rebuild, or what was being
   * served. When not, and it was not rewritten, workers refuse it until an
   * admin restores the right version or forces a rebuild.
   */
  object_accounted_for: boolean;
}

export interface ApiKey {
  id: string;
  label: string | null;
  is_active: boolean;
  created_at: string;
  last_used_at: string | null;
}

export interface Me {
  id: string;
  username: string;
  email: string;
  is_admin: boolean;
  tasks_completed: number;
}

// --- Endpoints -------------------------------------------------------------

export const api = {
  // Auth
  register: (body: { username: string; email: string; password: string }) =>
    post<{ message: string; mail_in_server_log?: boolean }>('/api/auth/register', body),
  login: (body: { username: string; password: string }) =>
    post<{ username: string; is_admin: boolean }>('/api/auth/login', body),
  logout: () => post<void>('/api/auth/logout'),
  signOutEverywhere: () => post<void>('/api/auth/sign-out-everywhere'),
  confirmEmail: (code: string) => post<{ message: string }>('/api/auth/confirm-email', { code }),
  requestPasswordReset: (email: string) =>
    post<{ message: string }>('/api/auth/reset-password/request', { email }),
  confirmPasswordReset: (token: string, password: string) =>
    post<{ message: string }>('/api/auth/reset-password/confirm', { token, password }),

  // Account
  me: () => get<Me>('/api/me'),
  apiKeys: () => get<ApiKey[]>('/api/me/api-keys'),
  createApiKey: (label: string | null) =>
    post<{ id: string; label: string | null; key: string }>('/api/me/api-keys', { label }),
  setApiKeyActive: (id: string, is_active: boolean) =>
    patch<void>(`/api/me/api-keys/${id}`, { is_active }),
  revokeApiKey: (id: string) => del<void>(`/api/me/api-keys/${id}`),

  // Public
  jobConfig: (id: string) => get<import('$lib/jobSettings').JobConfig>(`/api/jobs/${id}/config`),
  jobs: (page = 0, status?: JobStatus) =>
    get<Page<JobListItem>>(`/api/jobs?page=${page}${status ? `&status=${status}` : ''}`),
  job: (id: string) => get<JobStats>(`/api/jobs/${id}`),
  /** Up to `n` distinct racks an opening-rack job has analysed, drawn at random. */
  rackSamples: (id: string, n = 10) =>
    get<{ racks: string[] }>(`/api/jobs/${id}/rack-samples?${query({ n })}`),
  /**
   * One rack's ranked moves in an opening-rack job, in one page: every analysis
   * of it, each with its best moves -- the whole list when there are few
   * analyses, and fewer per analysis the more there are (32,767 moves in all).
   */
  rackLookup: (id: string, rack: string) =>
    get<CursorPage<RackLookupRow>>(`/api/jobs/${id}/results?${new URLSearchParams({ rack })}`),
  /** Signed-in users only: a games or pairs job's captured positions with one rack, newest first. */
  jobPositions: (id: string, rack: string, params: { per_page?: number; cursor?: string } = {}) =>
    get<CursorPage<SavedPosition>>(
      `/api/jobs/${id}/positions?${query({ rack, ...params })}`
    ),
  /** Signed-in users only: one captured position at random, or `null` before any. */
  randomPosition: (id: string) => get<SavedPosition | null>(`/api/jobs/${id}/positions/random`),
  jobBoard: (id: string) => get<BoardData>(`/api/jobs/${id}/board`),
  publicPlayerConfigs: () => get<import('$lib/jobSettings').PublicPlayerConfig[]>('/api/player-configs'),
  publicPlayerConfig: (id: string) => get<import('$lib/jobSettings').PublicPlayerConfig>(`/api/player-configs/${id}`),
  ratingPools: () => get<RatingPoolListItem[]>('/api/rating-pools'),
  ratingPool: (id: string) => get<RatingPoolDetail>(`/api/rating-pools/${id}`),

  users: (page = 0) => get<Page<Record<string, unknown>>>(`/api/users?page=${page}`),
  workers: (page = 0, sort: ContributorSort = 'movegens') =>
    get<Page<Contributor>>(`/api/workers?page=${page}&sort=${sort}`),

  clientVersion: () =>
    get<{ min_magpie_version: string; download_url: string }>('/api/worker/client-version'),

  // Admin
  playerConfigs: () => get<PlayerConfig[]>('/api/admin/player-configs'),
  createPlayerConfig: (body: Record<string, unknown>) =>
    post<PlayerConfig>('/api/admin/player-configs', body),
  deletePlayerConfig: (id: string) => del<void>(`/api/admin/player-configs/${id}`),

  // Admin: rating pools. Membership is an admin decision because not every
  // player config belongs in a rating, and every change refits the whole pool.
  createRatingPool: (body: {
    name: string;
    variant: string;
    letterdist_id: string;
    layout_id: string;
    anchor_player_config_id: string;
    anchor_rating?: number;
  }) => post<{ id: string }>('/api/admin/rating-pools', body),
  /** `run_id` is null when the config is already a member: nothing is
   *  logged or refitted. */
  addRatingPoolMember: (poolId: string, player_config_id: string) =>
    post<{ run_id: string | null }>(`/api/admin/rating-pools/${poolId}/members`, {
      player_config_id
    }),
  removeRatingPoolMember: (poolId: string, configId: string) =>
    del<{ run_id: string }>(`/api/admin/rating-pools/${poolId}/members/${configId}`),
  recomputeRatingPool: (poolId: string) =>
    post<{ run_id: string }>(`/api/admin/rating-pools/${poolId}/recompute`),
  /** Moves the anchor (added as a member if it is not one) or its rating, and
   *  refits; `run_id` is null when nothing changed. */
  updateRatingPool: (
    poolId: string,
    body: { anchor_player_config_id?: string; anchor_rating?: number }
  ) => patch<{ run_id: string | null }>(`/api/admin/rating-pools/${poolId}`, body),
  deleteRatingPool: (poolId: string) => del<void>(`/api/admin/rating-pools/${poolId}`),
  inputData: () => get<InputData[]>('/api/admin/input-data'),
  deleteInputData: (id: string) => del<void>(`/api/admin/input-data/${id}`),
  startImport: (body: { tarball_date: string; git_ref?: string }) =>
    post<{ id: string; state: string }>('/api/admin/input-data/imports', body),
  getImport: (id: string) => get<ImportDetail>(`/api/admin/input-data/imports/${id}`),
  confirmImport: (id: string) =>
    post<{ inserted: number }>(`/api/admin/input-data/imports/${id}/confirm`),
  jobDataGaps: (id: string) => get<DataGap[]>(`/api/admin/jobs/${id}/data-gaps`),
  jobDerivedData: (id: string) =>
    get<JobDerivedFile[]>(`/api/admin/jobs/${id}/derived-data`),
  fleet: () => get<FleetVersion[]>('/api/admin/fleet'),
  backups: () => get<BackupStatus>('/api/admin/backups'),
  settings: () => get<Settings>('/api/admin/settings'),
  /** Applies to the claims made from now on; one that changes nothing writes nothing. */
  updateSettings: (body: { max_task_seconds: number }) => put<Settings>('/api/admin/settings', body),
  derivedData: () => get<DerivedData[]>('/api/admin/derived-data'),
  retryDerivedData: (row: DerivedData) =>
    post<void>('/api/admin/derived-data/retry', {
      role: row.role,
      name: row.name,
      builder: row.builder,
      kwg_id: row.kwg_id,
      klv_id: row.klv_id,
      letterdist_id: row.letterdist_id
    }),
  /**
   * A snapshot while the job is not completed; `409` for a completed job with
   * claims still in flight, or while an export of the job is already running.
   */
  startExport: (id: string) =>
    post<{ id: string; state: string }>(`/api/admin/jobs/${id}/export`),
  /** The newest export; `404` when the job has never been exported. */
  jobExport: (id: string) => get<JobExport>(`/api/admin/jobs/${id}/export`),
  /** Leave generation: fold staged results into the rack totals now rather than at the next sweep. */
  mergeLeaveProgress: (id: string) =>
    post<{ folds_merged: number; racks_updated: number }>(
      `/api/admin/jobs/${id}/merge-progress`
    ),
  rebuildArtifacts: (id: string, force = false) =>
    post<ArtifactRebuild[]>(`/api/admin/jobs/${id}/rebuild-artifacts?force=${force}`),
  /**
   * Every job the request made, inactive at 0%: one, or for a games or pairs
   * request naming n ≥ 2 player configs, one per pairing.
   */
  createJob: (body: Record<string, unknown>) =>
    post<{ jobs: JobRow[] }>('/api/admin/jobs', body),
  /**
   * Several jobs' allocations at once, the active jobs checked against 100%
   * as they will stand: above 0% activates a job, 0% deactivates one, and a
   * job not named keeps what it has. Nothing changes unless all of it does.
   * The only way a job is activated or deactivated.
   */
  setAllocations: (rows: { job_id: string; allocation: number }[]) =>
    put<{ jobs: JobRow[] }>('/api/admin/jobs/allocations', { allocations: rows }),
  completeJob: (id: string) => post<JobRow>(`/api/admin/jobs/${id}/complete`),
  /**
   * An opening-rack job's consensus settings, changed: only the fields given.
   * The job follows -- a completed one with racks unsettled again reopens,
   * inactive at 0% until it is given an allocation, and an active one with
   * every rack settled completes.
   */
  updateConsensus: (
    id: string,
    body: { min_results_per_rack?: number; max_results_per_rack?: number; consensus_pct?: number }
  ) =>
    patch<{
      job: JobRow;
      unsettled_racks: number;
      reopened: boolean;
    }>(`/api/admin/jobs/${id}/consensus`, body),
  purgeJob: (id: string) => post<{ tasks_reset: number }>(`/api/admin/jobs/${id}/purge`),
  deleteJob: (id: string) => del<void>(`/api/admin/jobs/${id}`),
  deleteUser: (id: string) => del<void>(`/api/admin/users/${id}`),
  /** Like `workers`, plus anonymous workers' UUIDs, which a ban needs. */
  adminWorkers: (page = 0) =>
    get<Page<Contributor>>(`/api/admin/workers?page=${page}`),
  banWorker: (body: { user_id?: string; anon_uuid?: string; reason?: string }) =>
    post<{ id: string }>('/api/admin/workers/ban', body),
  unbanWorker: (id: string) => del<void>(`/api/admin/workers/ban/${id}`),
  workerBans: () => get<WorkerBan[]>('/api/admin/workers/bans'),
  auditLog: (params: Record<string, string | number | undefined> = {}) =>
    get<Page<Record<string, unknown>>>(
      `/api/admin/audit-log?${query(params)}`
    )
};


// --- Ratings ---------------------------------------------------------------
//
// Ratings are pool-scoped, not job-scoped: a rating is a statement about a
// player config across every game pair it has played under one set of
// conditions, so it does not belong to any single job.

export interface RatingPoolListItem {
  id: string;
  name: string;
  variant: string;
  letter_distribution: string;
  layout: string;
  members: number;
  last_computed_at: string | null;
}

export interface RatingRow {
  player_config_id: string;
  name: string;
  rating: number;
  /**
   * Approximate standard error, in rating points (WESPA's scale: a gap of
   * 250·ln 3 ≈ 275 is a 75% score). Wide bars mean "barely measured".
   */
  stderr: number;
  pairs_played: number;
  /**
   * False when no chain of games links this config to the pool's anchor. Its
   * rating is then an artefact of the fit's prior and must be shown as unrated
   * rather than as a number.
   */
  connected_to_anchor: boolean;
  is_anchor: boolean;
}

/**
 * One cell of a pool's cross table, from the row config's side. The API
 * serves every head-to-head from both sides: the mirror has `1 - actual`,
 * `1 - predicted` and `-spread`, and the same `stderr`.
 */
export interface RatingHeadToHead {
  row: string;
  col: string;
  pairs: number;
  /** The row config's score per game, (W + ½D) / games, 0 to 1. */
  actual: number;
  /** What the fitted ratings predict `actual` to be: the gap is the residual. */
  predicted: number;
  /** The standard error of `actual`, from the pairs' score variance. */
  stderr: number;
  /** The row config's average spread per game: its game score minus the other's. */
  spread: number;
}

export interface RatingRun {
  id: string;
  computed_at: string;
  trigger: string;
  iterations: number;
  converged: boolean;
  pairs_used: number;
  jobs_used: number;
}

/** A config in a rating pool now, whether or not a fit has rated it yet. */
export interface RatingPoolMember {
  player_config_id: string;
  name: string;
}

export interface RatingPoolDetail {
  id: string;
  name: string;
  variant: string;
  letter_distribution: string;
  layout: string;
  anchor_player_config_id: string;
  anchor_rating: number;
  /**
   * The pool's members now, by name, the anchor among them. Not the set
   * `ratings` covers, which is the latest fit's: a config added since (or
   * whose refit failed) has no rating yet, and one removed since still has.
   */
  members: RatingPoolMember[];
  run: RatingRun | null;
  ratings: RatingRow[];
  head_to_heads: RatingHeadToHead[];
}

