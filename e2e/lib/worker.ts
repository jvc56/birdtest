import { expect, type APIRequestContext } from '@playwright/test';

/**
 * A worker the test drives itself, speaking the same protocol as
 * worker/fake_worker.py and `magpie contribute`. Used where a journey needs to
 * know *which* worker it is looking at, which the compose fake workers never
 * say.
 */

const CLAIM = { magpie_version: '99.0.0', unsupported_jobs: [] as string[] };

export interface Assignment {
  worker_uuid: string;
  claim_token: string;
  job_id: string;
  task_request: { job_type: string; num_games: number };
}

function aggregate(games: number, wins: number, losses: number) {
  return {
    games,
    wins,
    losses,
    ties: games - wins - losses,
    p1_score_mean: 420,
    p1_score_sd: 55,
    p2_score_mean: 415,
    p2_score_sd: 55
  };
}

/**
 * A plausible result for a games or game-pairs task: every pair split 1-1, as
 * two identical games do, and half of a games task's games won.
 */
export function syntheticResult(request: Assignment['task_request']) {
  const n = request.num_games;
  if (request.job_type === 'games') {
    const wins = Math.floor(n / 2);
    return { all_games: aggregate(n, wins, n - wins) };
  }
  if (request.job_type === 'game_pairs') {
    return { all_games: aggregate(2 * n, n, n), pentanomial: [0, 0, n, 0, 0] };
  }
  throw new Error(`no synthetic result for a ${request.job_type} task`);
}

/**
 * A brand-new anonymous worker's first claim, which is how an identity is
 * issued. Retried through "no work right now": the fake workers are taking
 * tasks from the same jobs.
 */
export async function firstClaim(api: APIRequestContext): Promise<Assignment> {
  let assignment: Assignment | undefined;
  await expect
    .poll(
      async () => {
        const response = await api.post('/api/worker/task', { data: CLAIM });
        if (response.status() === 200) {
          const answer = await response.json();
          if (answer.claim_token) assignment = answer as Assignment;
        }
        return assignment !== undefined;
      },
      { timeout: 60_000, intervals: [1000] }
    )
    .toBe(true);
  return assignment!;
}

export async function submit(api: APIRequestContext, uuid: string, assignment: Assignment) {
  return api.post('/api/worker/result', {
    headers: { 'X-Worker-UUID': uuid },
    data: { claim_token: assignment.claim_token, result: syntheticResult(assignment.task_request) }
  });
}

export async function claimAs(api: APIRequestContext, uuid: string) {
  return api.post('/api/worker/task', { headers: { 'X-Worker-UUID': uuid }, data: CLAIM });
}
