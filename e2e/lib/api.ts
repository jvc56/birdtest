import { expect, request, type APIRequestContext, type APIResponse } from '@playwright/test';
import { ADMIN_STATE, env, SEEDED_DATA } from './env';
import { CONFIRM_SUBJECT, linkIn, waitForMail } from './mail';

/**
 * The HTTP API, for the set-up a journey needs but is not about. A journey
 * that creates a job to have a job does it here; the journey that is *about*
 * creating a job does it in the browser.
 */

async function body<T>(response: APIResponse, what: string): Promise<T> {
  if (!response.ok()) {
    throw new Error(`${what}: ${response.status()} ${await response.text()}`);
  }
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

/** A session as the seeded admin, with the CSRF header every write needs. */
export class AdminApi {
  private constructor(
    private readonly ctx: APIRequestContext,
    private readonly csrf: string
  ) {}

  static async open(): Promise<AdminApi> {
    const ctx = await request.newContext({ baseURL: env.baseURL, storageState: ADMIN_STATE });
    const state = await ctx.storageState();
    const csrf = state.cookies.find((cookie) => cookie.name === 'birdtest_csrf')?.value ?? '';
    return new AdminApi(ctx, decodeURIComponent(csrf));
  }

  async get<T>(path: string): Promise<T> {
    return body<T>(await this.ctx.get(path), `GET ${path}`);
  }

  async post<T>(path: string, data: unknown = {}): Promise<T> {
    const response = await this.ctx.post(path, { data, headers: { 'x-csrf-token': this.csrf } });
    return body<T>(response, `POST ${path}`);
  }

  async dispose() {
    await this.ctx.dispose();
  }

  /** The seeded rows by role and name, from the tarball the seed imported. */
  async seededData() {
    const rows = await this.get<{ id: string; role: string; name: string; tarball_date: string }[]>(
      '/api/admin/input-data'
    );
    const find = (role: string, name: string) => {
      const row = rows.find((r) => r.role === role && r.name === name && r.tarball_date === SEEDED_DATA);
      if (!row) throw new Error(`no seeded ${role} ${name}`);
      return row.id;
    };
    return {
      kwg: find('kwg', 'NWL23'),
      klv: find('klv', 'NWL23'),
      letterdist: find('letterdist', 'english_fixture'),
      layout: find('layout', 'standard15')
    };
  }

  async playerConfigId(name: string): Promise<string> {
    const configs = await this.get<{ id: string; name: string }[]>('/api/admin/player-configs');
    const config = configs.find((c) => c.name === name);
    if (!config) throw new Error(`no player config ${name}`);
    return config.id;
  }

  /** A static player on the seeded data, which no wordmap or table gates. */
  async createStaticConfig(name: string, sortStrategy: 'equity' | 'score', plays = 10) {
    const data = await this.seededData();
    const created = await this.post<{ id: string }>('/api/admin/player-configs', {
      name,
      recorder_type: 'best',
      sort_strategy: sortStrategy,
      kwg_id: data.kwg,
      klv_id: data.klv,
      num_plays_recorded: plays
    });
    return created.id;
  }

  /** Creates and activates a job on the seeded data; returns its id. */
  async activeJob(config: Record<string, unknown>, allocation: number): Promise<string> {
    const data = await this.seededData();
    const created = await this.post<{ job: { id: string } }>('/api/admin/jobs', {
      variant: 'classic',
      letterdist_id: data.letterdist,
      layout_id: data.layout,
      redundancy: 1,
      ...config
    });
    await this.post(`/api/admin/jobs/${created.job.id}/activate`, { allocation });
    return created.job.id;
  }
}

export interface JobSummary {
  id: string;
  job_type: string;
  status: string;
  created_at: string;
}

/** The game-pairs job scripts/seed.py created: the first one there is. */
export async function seededJob(api: APIRequestContext): Promise<JobSummary> {
  const page = await body<{ items: JobSummary[] }>(await api.get('/api/jobs?per_page=500'), 'list jobs');
  const pairs = page.items
    .filter((job) => job.job_type === 'game_pairs')
    .sort((a, b) => a.created_at.localeCompare(b.created_at));
  if (!pairs.length) throw new Error('no seeded game-pairs job; was the stack seeded?');
  return pairs[0];
}

/**
 * Waits until a job has finished and nothing is still in flight on it, so
 * its evidence can no longer move: a result claimed before the verdict can
 * still land after it.
 */
export async function waitUntilSettled(api: APIRequestContext, jobId: string, timeoutMs = 240_000) {
  await expect
    .poll(
      async () => {
        const stats = await body<{ job: { status: string }; tasks_claimed: number }>(
          await api.get(`/api/jobs/${jobId}`),
          'job stats'
        );
        return `${stats.job.status}, ${stats.tasks_claimed} claimed`;
      },
      { timeout: timeoutMs, intervals: [1000] }
    )
    .toBe('completed, 0 claimed');
}

/** Waits until a job has accepted at least one result. */
export async function waitForResults(api: APIRequestContext, jobId: string, timeoutMs = 120_000) {
  await expect
    .poll(
      async () =>
        (await body<{ results_accepted: number }>(await api.get(`/api/jobs/${jobId}`), 'job stats'))
          .results_accepted,
      { timeout: timeoutMs, intervals: [1000] }
    )
    .toBeGreaterThan(0);
}

/**
 * A registered, confirmed account, made through the API and the outbox.
 * For journeys that need *a user* rather than journeys about registering.
 */
export async function confirmedUser(user: { username: string; email: string; password: string }) {
  const ctx = await request.newContext({ baseURL: env.baseURL });
  try {
    await body(await ctx.post('/api/auth/register', { data: user }), 'register');
    const mail = await waitForMail(user.email, CONFIRM_SUBJECT);
    const code = new URL(linkIn(mail, '/confirm-email')).searchParams.get('code');
    await body(await ctx.post('/api/auth/confirm-email', { data: { code } }), 'confirm email');
  } finally {
    await ctx.dispose();
  }
}
