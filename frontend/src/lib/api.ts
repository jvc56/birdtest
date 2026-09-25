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
    readonly fields: Record<string, string> = {}
  ) {
    super(message);
  }
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
      fields
    );
  }
  return payload as T;
}

const get = <T>(path: string) => request<T>('GET', path);
const post = <T>(path: string, body?: unknown) => request<T>('POST', path, body ?? {});
const patch = <T>(path: string, body: unknown) => request<T>('PATCH', path, body);
const del = <T>(path: string) => request<T>('DELETE', path);

// --- Shared shapes ---------------------------------------------------------

export type JobType = 'opening_rack' | 'games' | 'game_pairs' | 'leave_generation';
export type JobStatus = 'active' | 'inactive' | 'completed';

export interface Page<T> {
  items: T[];
  total: number;
  page: number;
  per_page: number;
}

/**
 * A job's results page by cursor rather than by offset: the corpus runs to
 * millions of rows, where `OFFSET` produces every row before the page asked
 * for. Pass `next_cursor` back as `cursor` for the next page; its absence is
 * the end. This is the only endpoint that pages this way.
 */
export interface CursorPage<T> {
  items: T[];
  /** Always -1: an exact count costs more than it is worth to the caller. */
  total: number;
  per_page: number;
  next_cursor?: string;
}

export interface JobListItem {
  id: string;
  job_type: JobType;
  status: JobStatus;
  /** The job's share of the fleet while active; null until first activated. 0% means what inactive means. */
  allocation: number | null;
  redundancy: number;
  created_at: string;
  tasks_total: number;
  tasks_completed: number;
  units_completed: number | null;
  max_units: number | null;
  /** Workers are declining this job and none is completing it — usually data nobody has. */
  stalled: boolean;
}

/**
 * A job's own row, as the admin actions (create, activate, deactivate,
 * complete) return it -- not the list's summary, which adds counts.
 */
export interface JobRow {
  id: string;
  job_type: JobType;
  status: JobStatus;
  allocation: number | null;
  redundancy: number;
  variant: string;
  created_at: string;
}

export interface SprtResult {
  llr: number;
  lower_bound: number;
  upper_bound: number;
  status: 'running' | 'passed' | 'failed' | 'terminated_at_max';
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
   * Game pairs only: the five pair outcomes the LLR is computed from, indexed
   * by player 1's half-point score across the pair (0 = lost both, 4 = won
   * both). Every completed pair is in here, including the ones that played
   * identically — they are 1-1 ties in bucket 2, and they are what makes a
   * paired run lower-variance than an unpaired one.
   */
  pentanomial?: [number, number, number, number, number];
  /** Game pairs only: how many pairs diverged. A diagnostic, not the sample. */
  divergent_pairs?: number;
  min_units: number;
  max_units: number;
  win_pct: number;
  loss_pct: number;
  draw_pct: number;
  /** The test over every accepted result, recomputed on each read. */
  sprt: SprtResult;
  /**
   * What a completed job stopped on, when the finish check completed it. Results
   * in flight at that moment still land, so `sprt` can move afterwards; this is
   * the decision that stands.
   */
  decided?: { status: SprtResult['status']; llr: number; units: number };
}

export interface JobStats {
  job: {
    id: string;
    job_type: JobType;
    status: JobStatus;
    allocation: number | null;
    redundancy: number;
    min_magpie_version: string;
    created_at: string;
    created_by: string | null;
    /** The lexicons in play. A games job comparing two reads "CSW21 vs NWL23". */
    lexicon: string | null;
    variant: string | null;
  };
  tasks_total: number;
  tasks_completed: number;
  tasks_available: number;
  tasks_claimed: number;
  results_accepted: number;
  games?: GameStats;
  opening_racks?: {
    racks_analyzed: number;
    /** Size of the rack space — the denominator for progress. */
    racks_total: number;
  };
  leave_generation?: {
    current_generation: number;
    /** Generations whose KLV is built; `current_generation` stops at the last one. */
    generations_closed: number;
    generation_count: number;
    target_rack_count: number;
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
  }[];
  /** Contributors beyond the ones listed; the list is capped. */
  other_workers: number;
  eta_seconds: number | null;
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
  min_play_iterations: number | null;
  threshold: string | null;
  sampling_rule: string | null;
  inference_margin: number | null;
  utility_w_winpct: number | null;
  utility_w_spread: number | null;
  utility_spread_scale: number | null;
  movegen_margin: number;
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
  state: 'running' | 'staged' | 'confirmed' | 'cancelled' | 'failed';
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

/**
 * A wordmap or rack info table the server builds a reference copy of.
 *
 * Neither file is shipped — 179 MB and 1.9 GB for CSW24 — so what travels to a
 * worker is the SHA-256 the server's own pinned MAGPIE got from the same
 * inputs. A job that needs one is not dispatched until it is `built`, which is
 * why this page exists: an active job doing nothing usually has a row here.
 */
export interface DerivedData {
  /** `wmp` or `rit`. */
  role: string;
  /** What the worker loads it as: a lexicon, or `<lexicon>.<leaves>`. */
  name: string;
  /** The builder that produced the hash, e.g. `wmp-1`. */
  builder: string;
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
    post<{ message: string }>('/api/auth/register', body),
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
  jobs: (page = 0, status?: JobStatus) =>
    get<Page<JobListItem>>(`/api/jobs?page=${page}${status ? `&status=${status}` : ''}`),
  job: (id: string) => get<JobStats>(`/api/jobs/${id}`),
  /** Cursor-paginated; see {@link CursorPage}. `?rack=` returns one rack's whole list. */
  jobResults: (id: string, params: Record<string, string | number> = {}) =>
    get<CursorPage<Record<string, unknown>>>(
      `/api/jobs/${id}/results?${new URLSearchParams(
        Object.entries(params).map(([k, v]) => [k, String(v)])
      )}`
    ),
  ratingPools: () => get<RatingPoolListItem[]>('/api/rating-pools'),
  ratingPool: (id: string) => get<RatingPoolDetail>(`/api/rating-pools/${id}`),
  ratingHistory: (id: string) => get<RatingHistoryPoint[]>(`/api/rating-pools/${id}/history`),

  users: (page = 0) => get<Page<Record<string, unknown>>>(`/api/users?page=${page}`),
  workers: (page = 0) => get<Page<Record<string, unknown>>>(`/api/workers?page=${page}`),

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
  addRatingPoolMember: (poolId: string, player_config_id: string) =>
    post<{ run_id: string }>(`/api/admin/rating-pools/${poolId}/members`, { player_config_id }),
  removeRatingPoolMember: (poolId: string, configId: string) =>
    del<{ run_id: string }>(`/api/admin/rating-pools/${poolId}/members/${configId}`),
  recomputeRatingPool: (poolId: string) =>
    post<{ run_id: string }>(`/api/admin/rating-pools/${poolId}/recompute`),
  inputData: () => get<InputData[]>('/api/admin/input-data'),
  deleteInputData: (id: string) => del<void>(`/api/admin/input-data/${id}`),
  startImport: (body: { tarball_date: string; git_ref?: string }) =>
    post<{ id: string; state: string }>('/api/admin/input-data/imports', body),
  getImport: (id: string) => get<ImportDetail>(`/api/admin/input-data/imports/${id}`),
  confirmImport: (id: string) =>
    post<{ inserted: number }>(`/api/admin/input-data/imports/${id}/confirm`),
  jobDataGaps: (id: string) => get<DataGap[]>(`/api/admin/jobs/${id}/data-gaps`),
  fleet: () => get<FleetVersion[]>('/api/admin/fleet'),
  backups: () => get<BackupStatus>('/api/admin/backups'),
  derivedData: () => get<DerivedData[]>('/api/admin/derived-data'),
  retryDerivedData: (role: string, name: string) =>
    post<void>('/api/admin/derived-data/retry', { role, name }),
  /** `409` unless the job is completed and its last claims have landed. */
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
  createJob: (body: Record<string, unknown>) =>
    post<{ job: JobRow }>('/api/admin/jobs', body),
  activateJob: (id: string, allocation: number) =>
    post<JobRow>(`/api/admin/jobs/${id}/activate`, { allocation }),
  deactivateJob: (id: string) => post<JobRow>(`/api/admin/jobs/${id}/deactivate`),
  completeJob: (id: string) => post<JobRow>(`/api/admin/jobs/${id}/complete`),
  purgeJob: (id: string) => post<{ tasks_reset: number }>(`/api/admin/jobs/${id}/purge`),
  deleteJob: (id: string) => del<void>(`/api/admin/jobs/${id}`),
  deleteUser: (id: string) => del<void>(`/api/admin/users/${id}`),
  /** Like `workers`, plus anonymous workers' UUIDs, which a ban needs. */
  adminWorkers: (page = 0) =>
    get<Page<Record<string, unknown>>>(`/api/admin/workers?page=${page}`),
  banWorker: (body: { user_id?: string; anon_uuid?: string; reason?: string }) =>
    post<{ id: string }>('/api/admin/workers/ban', body),
  unbanWorker: (id: string) => del<void>(`/api/admin/workers/ban/${id}`),
  workerBans: () => get<WorkerBan[]>('/api/admin/workers/bans'),
  auditLog: (params: Record<string, string | number> = {}) =>
    get<Page<Record<string, unknown>>>(
      `/api/admin/audit-log?${new URLSearchParams(
        Object.entries(params).map(([k, v]) => [k, String(v)])
      )}`
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
  /** Approximate Elo standard error. Wide bars mean "barely measured". */
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

export interface RatingResidual {
  row: string;
  col: string;
  pairs: number;
  actual: number;
  predicted: number;
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

export interface RatingPoolDetail {
  id: string;
  name: string;
  variant: string;
  letter_distribution: string;
  layout: string;
  anchor_player_config_id: string;
  anchor_rating: number;
  run: RatingRun | null;
  ratings: RatingRow[];
  residuals: RatingResidual[];
}

export interface RatingHistoryPoint {
  computed_at: string;
  player_config_id: string;
  name: string;
  rating: number;
  stderr: number;
}
