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
  const payload = text ? JSON.parse(text) : undefined;

  if (!response.ok) {
    const fields: Record<string, string> = {};
    for (const entry of payload?.fields ?? []) fields[entry.field] = entry.message;
    throw new ApiError(
      response.status,
      payload?.code ?? 'error',
      payload?.message ?? response.statusText,
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

export interface JobListItem {
  id: string;
  job_type: JobType;
  status: JobStatus;
  priority: number;
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
  sprt: SprtResult;
}

export interface JobStats {
  job: {
    id: string;
    job_type: JobType;
    status: JobStatus;
    priority: number;
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
    average_best_equity: number | null;
    best_move_types: { move_type: string; count: number }[];
  };
  leave_generation?: {
    current_generation: number;
    generation_count: number;
    target_rack_count: number;
    racks_at_target: number;
    racks_total: number;
    min_rack: string | null;
    min_rack_count: number | null;
  };
  workers: {
    user_id: string | null;
    /** An anonymous worker's public pseudonym; its UUID is never published. */
    anon_id: string | null;
    username: string | null;
    tasks_completed: number;
  }[];
  eta_seconds: number | null;
}

export interface PlayerConfig {
  id: string;
  name: string;
  recorder_type: string;
  sort_strategy: string | null;
  /** The files this player pins, as input_data rows rather than names. */
  kwg_id: string;
  klv_id: string;
  /** Null for a static player, which never loads a win% model. */
  winpct_id: string | null;
  /** Set when this config was cloned onto newer data; a clone starts unrated. */
  cloned_from_id: string | null;
  max_iterations: number | null;
  num_plies: number | null;
  num_plies_recorded: number | null;
  num_plays: number | null;
  num_plays_recorded: number;
  stopping_pct: number | null;
  use_inference: boolean | null;
  time_limit_secs: number | null;
  use_wordmap: boolean | null;
  use_rit: boolean | null;
  min_play_iterations: number | null;
  threshold: string | null;
  sampling_rule: string | null;
  inference_margin: number | null;
  utility_w_winpct: number | null;
  utility_w_spread: number | null;
  utility_spread_scale: number | null;
  movegen_margin: number | null;
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

/** Per generation, what rebuilding a leave job's KLV from the database found. */
export interface ArtifactRebuild {
  generation: number;
  artifact_key: string;
  stored_sha256: string;
  rebuilt_sha256: string;
  matches: boolean;
  object_present: boolean;
  rewritten: boolean;
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
  jobs: (page = 0) => get<Page<JobListItem>>(`/api/jobs?page=${page}`),
  job: (id: string) => get<JobStats>(`/api/jobs/${id}`),
  jobResults: (id: string, params: Record<string, string | number> = {}) =>
    get<Page<Record<string, unknown>>>(
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
  rebuildArtifacts: (id: string, force = false) =>
    post<ArtifactRebuild[]>(`/api/admin/jobs/${id}/rebuild-artifacts?force=${force}`),
  createJob: (body: Record<string, unknown>) =>
    post<{ job: JobListItem; initialized: number }>('/api/admin/jobs', body),
  activateJob: (id: string, allocation: number) =>
    post<JobListItem>(`/api/admin/jobs/${id}/activate`, { allocation }),
  deactivateJob: (id: string) => post<JobListItem>(`/api/admin/jobs/${id}/deactivate`),
  completeJob: (id: string) => post<JobListItem>(`/api/admin/jobs/${id}/complete`),
  purgeJob: (id: string) => post<{ tasks_reset: number }>(`/api/admin/jobs/${id}/purge`),
  deleteJob: (id: string) => del<void>(`/api/admin/jobs/${id}`),
  deleteUser: (id: string) => del<void>(`/api/admin/users/${id}`),
  /** Like `workers`, plus anonymous workers' UUIDs, which a ban needs. */
  adminWorkers: (page = 0) =>
    get<Page<Record<string, unknown>>>(`/api/admin/workers?page=${page}`),
  banWorker: (body: { user_id?: string; anon_uuid?: string; reason?: string }) =>
    post<{ id: string }>('/api/admin/workers/ban', body),
  unbanWorker: (id: string) => del<void>(`/api/admin/workers/ban/${id}`),
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
