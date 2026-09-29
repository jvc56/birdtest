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

  // And an action's read leaves it too: only Activate sends it (Deactivate's
  // read put 7 back, and the next Activate sent that).
  await page.route(`**/api/admin/jobs/${id}/deactivate`, (route) => route.fulfill({ status: 204, body: '' }));
  await page.getByRole('button', { name: 'Deactivate', exact: true }).click();
  await expect(page.getByText('Job deactivated.')).toBeVisible();
  await expect(page.locator('#alloc')).toHaveValue('55');
  await expect(page.getByText(/Set: 7% \(offered to nobody while inactive\)/)).toBeVisible();
  await api.dispose();
});

/**
 * E-11b: a read started before a live payload does not land over it. A retry
 * whose job read was slow put back the status the stream had moved past, and
 * a completed job sends nothing to correct it (thirty-second audit, pass 17).
 */
test('E-11b: a slow read does not undo what the stream has since said', async ({ page }) => {
  const api = await AdminApi.open();
  const a = await api.createStaticConfig(`e11b-a-${Date.now()}`, 'equity');
  const b = await api.createStaticConfig(`e11b-b-${Date.now()}`, 'score');
  const id = await api.activeJob(
    {
      job_type: 'game_pairs',
      player1_config_id: a,
      player2_config_id: b,
      pairs_per_batch: 1,
      min_pairs: 100000,
      max_pairs: 200000
    },
    3
  );
  await api.post(`/api/admin/jobs/${id}/deactivate`);
  const base = await api.get<{ job: Record<string, unknown> }>(`/api/jobs/${id}`);
  const as = (status: string) => ({ ...base, job: { ...base.job, status, allocation: 7 } });
  let current = as('active');

  // The stream's second connection says the job completed; the page's second
  // job read starts before that and is held until it has been said.
  let saidCompleted: () => void = () => {};
  const completedSaid = new Promise<void>((resolve) => (saidCompleted = resolve));
  let streams = 0;
  await page.route(`**/api/jobs/${id}/stream`, (route) => {
    streams += 1;
    if (streams >= 2) current = as('completed');
    const body = `retry: ${streams === 1 ? 9000 : 600000}\nevent: stats\ndata: ${JSON.stringify(current)}\n\n`;
    const fulfilled = route.fulfill({ headers: { 'content-type': 'text/event-stream' }, body });
    if (streams >= 2) fulfilled.then(() => setTimeout(saidCompleted, 500));
    return fulfilled.catch(() => {});
  });
  let jobReads = 0;
  let heldStatus = '';
  await page.route(`**/api/jobs/${id}`, async (route) => {
    jobReads += 1;
    const snapshot = JSON.stringify(current);
    if (jobReads === 2) {
      heldStatus = current.job.status as string;
      await completedSaid;
    }
    return route.fulfill({ contentType: 'application/json', body: snapshot }).catch(() => {});
  });
  // The first gap read fails, so the page retries (after five seconds).
  let gapReads = 0;
  await page.route(`**/api/admin/jobs/${id}/data-gaps`, (route) =>
    ++gapReads === 1 ? route.fulfill({ status: 503, body: '' }) : route.fulfill({ json: [] })
  );

  await page.goto(`/admin/jobs/${id}`);
  await expect(page.getByText(/Could not load all of this job/)).toBeVisible();
  await expect.poll(() => jobReads, { timeout: 20_000 }).toBeGreaterThanOrEqual(2);
  await completedSaid;
  // The held read did carry the old status, or this proves nothing.
  expect(heldStatus).toBe('active');
  await expect(page.getByText(/Could not load all of this job/)).toBeHidden({ timeout: 10_000 });
  const status = page.getByTestId('job-status');
  await expect(status.getByText('completed', { exact: true })).toBeVisible();
  await page.waitForTimeout(1000);
  await expect(status.getByText('completed', { exact: true })).toBeVisible();
  await api.dispose();
});

/**
 * E-11c: a job deleted while its page is open -- by another admin, say -- is
 * said to be gone and offered no more. Only a failed read noticed before, so
 * every action stayed, each answering "no such job" (thirty-second audit,
 * pass 18).
 */
test('E-11c: a job deleted while its page is open offers nothing more', async ({ page }) => {
  const api = await AdminApi.open();
  const a = await api.createStaticConfig(`e11c-a-${Date.now()}`, 'equity');
  const b = await api.createStaticConfig(`e11c-b-${Date.now()}`, 'score');
  const id = await api.activeJob(
    {
      job_type: 'game_pairs',
      player1_config_id: a,
      player2_config_id: b,
      pairs_per_batch: 1,
      min_pairs: 100000,
      max_pairs: 200000
    },
    3
  );
  await api.post(`/api/admin/jobs/${id}/deactivate`);
  await page.goto(`/admin/jobs/${id}`);
  await expect(page.getByRole('heading', { name: 'Controls' })).toBeVisible();

  await api.delete(`/api/admin/jobs/${id}`);
  await page.getByRole('button', { name: 'Deactivate', exact: true }).click();
  await expect(page.getByText('This job no longer exists.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Activate', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Delete job', exact: true })).toBeDisabled();
  await api.dispose();
});
