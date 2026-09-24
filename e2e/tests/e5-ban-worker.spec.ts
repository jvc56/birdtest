import { test, expect } from '@playwright/test';
import { AdminApi } from '../lib/api';
import { ADMIN_STATE, env } from '../lib/env';
import { claimAs, firstClaim, submit } from '../lib/worker';

test.use({ storageState: ADMIN_STATE });

let api: AdminApi;
let jobId: string;

// Work for the worker to claim that does not run out while the journey runs:
// a small share, a cap far beyond anything the suite reaches, taken back out
// of rotation afterwards.
test.beforeAll(async () => {
  api = await AdminApi.open();
  jobId = await api.activeJob(
    {
      job_type: 'games',
      player1_config_id: await api.playerConfigId('static-equity'),
      player2_config_id: await api.playerConfigId('static-score'),
      games_per_batch: 1,
      min_games: 100000,
      max_games: 100000
    },
    10
  );
});

test.afterAll(async () => {
  await api.post(`/api/admin/jobs/${jobId}/deactivate`);
  await api.dispose();
});

/**
 * E-5: an admin bans a worker, and that worker can no longer claim.
 *
 * The worker is one this journey runs itself -- claiming and submitting over
 * the worker API exactly as a contributor does -- so it knows which identity
 * it is watching, which the compose fake workers never say.
 */
test('E-5: an admin bans a worker and that worker can no longer claim', async ({ page, playwright }) => {
  const worker = await playwright.request.newContext({ baseURL: env.baseURL });
  try {
    // A new contributor: its first claim issues its identity, and one
    // finished task puts it on the admin's list of known workers.
    const assignment = await firstClaim(worker);
    const uuid = assignment.worker_uuid;
    const submitted = await submit(worker, uuid, assignment);
    expect(submitted.status(), await submitted.text()).toBe(200);
    expect((await submitted.json()).accepted).toBe(true);

    await page.goto('/admin/workers');
    await expect(page.getByRole('heading', { name: 'Worker bans' })).toBeVisible();
    const row = page.locator('tbody tr', { hasText: uuid });
    await expect(row).toContainText(/Anonymous · [0-9a-f]{8}/);
    await row.getByRole('button', { name: 'Select' }).click();
    await expect(page.getByLabel('User ID or anonymous UUID')).toHaveValue(uuid);
    await page.getByLabel('Reason').fill('e2e: submitting nonsense');
    await page.getByRole('button', { name: 'Ban worker' }).click();
    await expect(page.getByText(`Banned ${uuid}.`)).toBeVisible();

    // The banned identity is refused at the door.
    const refused = await claimAs(worker, uuid);
    expect(refused.status()).toBe(403);
    expect((await refused.json()).message).toBe('this worker identity is banned');

    // And it is on the record, with its reason.
    await page.goto('/admin/audit-log');
    const entry = page
      .locator('tbody tr', { hasText: 'worker.banned' })
      .filter({ hasText: `worker ${uuid.slice(0, 8)}` });
    await expect(entry).toContainText('e2e: submitting nonsense');
  } finally {
    await worker.dispose();
  }
});
