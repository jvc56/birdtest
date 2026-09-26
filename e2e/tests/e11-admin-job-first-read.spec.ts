import { test, expect } from '@playwright/test';
import { AdminApi } from '../lib/api';
import { ADMIN_STATE } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-11: the admin job page after its first read fails shows nothing the server
 * did not say. The reads went one after another, so a 503 on the first (a
 * deploy's, or the display pool's) left the data gaps and the allocation
 * unread while the stream filled in the stats: the page said "No worker has
 * declined this job" and showed an allocation of 100, and Activate sent the
 * 100 (thirty-second audit, pass 16). And a retry must not replace what the
 * admin has typed since: one did, and Activate sent the job's old value.
 */
test('E-11: an admin job page whose first read fails shows only what the server said', async ({ page }) => {
  const api = await AdminApi.open();
  const a = await api.createStaticConfig(`e11-a-${Date.now()}`, 'equity');
  const b = await api.createStaticConfig(`e11-b-${Date.now()}`, 'score');
  const id = await api.activeJob(
    {
      job_type: 'game_pairs',
      player1_config_id: a,
      player2_config_id: b,
      pairs_per_batch: 1,
      min_pairs: 100000,
      max_pairs: 200000
    },
    7
  );
  await api.post(`/api/admin/jobs/${id}/deactivate`);

  // The first read of the job fails once, and of its data gaps twice, so the
  // page retries while the admin types; a worker has declined it for data.
  let jobReads = 0;
  await page.route(`**/api/jobs/${id}`, async (route) => {
    if (route.request().method() !== 'GET') return route.continue();
    jobReads += 1;
    if (jobReads === 1) return route.fulfill({ status: 503, body: '' });
    return route.continue();
  });
  let gapReads = 0;
  await page.route(`**/api/admin/jobs/${id}/data-gaps`, (route) => {
    gapReads += 1;
    if (gapReads <= 2) return route.fulfill({ status: 503, body: '' });
    return route.fulfill({
      json: [
        {
          role: 'kwg',
          name: 'NWL23',
          expected: 'ab'.repeat(32),
          workers: 12,
          declines: 340,
          last_reported_at: new Date().toISOString()
        }
      ]
    });
  });
  const activations: string[] = [];
  await page.route(`**/api/admin/jobs/${id}/activate`, (route) => {
    activations.push(route.request().postData() ?? '');
    return route.fulfill({ status: 409, json: { code: 'conflict', message: 'mocked', fields: [] } });
  });

  await page.goto(`/admin/jobs/${id}`);
  await expect(page.getByRole('heading', { name: 'Controls' })).toBeVisible({ timeout: 20_000 });
  const gapsCard = page.locator('.card', { has: page.getByRole('heading', { name: 'Data gaps' }) });
  // Unread, the gaps are not "none"; the allocation is the job's, not 100.
  await expect(gapsCard).toContainText('Could not load the data gaps');
  await expect(page.locator('#alloc')).toHaveValue('7');

  // What the admin types survives the retries (one replaced it with 7).
  await page.locator('#alloc').fill('55');
  await expect(gapsCard.getByRole('cell', { name: /NWL23/ })).toBeVisible({ timeout: 20_000 });
  expect(gapReads).toBeGreaterThanOrEqual(3);
  expect(jobReads).toBeGreaterThanOrEqual(2);
  await expect(page.getByText(/Could not load all of this job/)).toBeHidden();
  await expect(page.locator('#alloc')).toHaveValue('55');

  await page.getByRole('button', { name: 'Activate', exact: true }).click();
  await expect.poll(() => activations.length).toBe(1);
  expect(JSON.parse(activations[0])).toEqual({ allocation: 55 });
  await api.dispose();
});
