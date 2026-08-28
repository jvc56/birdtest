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
}

export interface SprtResult {
  llr: number;
  lower_bound: number;
  upper_bound: number;
  status: 'running' | 'passed' | 'failed' | 'terminated_at_max';
}

export interface GameStats {
  unit: 'game' | 'pair';
  /** Counts over the tally the LLR is computed from — for pairs, the divergent subset. */
  wins: number;
  losses: number;
  draws: number;
  /** Games for a `games` job, pairs for a `game_pairs` job. */
  units_completed: number;
  /** Game pairs only: how many pairs diverged and so carried any signal. */
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
    min_magpie_version: string | null;
    created_at: string;
    created_by: string | null;
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
  ratings: {
    player_config_id: string;
    name: string;
    rating: number;
    rating_deviation: number;
    games_played: number;
  }[];
  workers: {
    user_id: string | null;
    anon_uuid: string | null;
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
  leaves: string | null;
  max_iterations: number | null;
  plies: number | null;
  num_plies_recorded: number | null;
  num_plays: number | null;
  num_plays_recorded: number | null;
  stopping_pct: number | null;
  use_inference: boolean | null;
  time_limit_secs: number | null;
  created_at: string;
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
  users: (page = 0) => get<Page<Record<string, unknown>>>(`/api/users?page=${page}`),
  workers: (page = 0) => get<Page<Record<string, unknown>>>(`/api/workers?page=${page}`),

  // Admin
  playerConfigs: () => get<PlayerConfig[]>('/api/admin/player-configs'),
  createPlayerConfig: (body: Record<string, unknown>) =>
    post<PlayerConfig>('/api/admin/player-configs', body),
  deletePlayerConfig: (id: string) => del<void>(`/api/admin/player-configs/${id}`),
  createJob: (body: Record<string, unknown>) =>
    post<{ job: JobListItem; prepopulated: number }>('/api/admin/jobs', body),
  activateJob: (id: string, allocation: number) =>
    post<JobListItem>(`/api/admin/jobs/${id}/activate`, { allocation }),
  deactivateJob: (id: string) => post<JobListItem>(`/api/admin/jobs/${id}/deactivate`),
  completeJob: (id: string) => post<JobListItem>(`/api/admin/jobs/${id}/complete`),
  purgeJob: (id: string) => post<{ tasks_reset: number }>(`/api/admin/jobs/${id}/purge`),
  deleteJob: (id: string) => del<void>(`/api/admin/jobs/${id}`),
  deleteUser: (id: string) => del<void>(`/api/admin/users/${id}`),
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
