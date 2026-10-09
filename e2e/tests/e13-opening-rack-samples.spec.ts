import { test, expect } from '@playwright/test';
import { AdminApi } from '../lib/api';
import { env } from '../lib/env';

/**
 * E-13: an opening-rack job's page offers racks it has analysed under the
 * search, drawn at random, and one click looks one up. Anyone may: the
 * lookup is public.
 */
let api: AdminApi;
let jobId: string;

test.beforeAll(async ({ playwright }) => {
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  jobId = await api.activeJob(
    {
      name: `e13 opening racks ${suffix}`,
      job_type: 'opening_rack',
      // One play recorded: a static `best` player records one move.
      player_config_id: await api.createStaticConfig(`e13-${suffix}`, 'equity', 1),
      racks_per_batch: 5,
      rack_size: 7
    },
    10
  );
  const request = await playwright.request.newContext({ baseURL: env.baseURL });
  // Ten distinct racks to draw: the job is small enough to be sampled whole,
  // so the page's own draw finds ten too.
  await expect
    .poll(
      async () => (await (await request.get(`/api/jobs/${jobId}/rack-samples?n=10`)).json()).racks.length,
      { timeout: 120_000, intervals: [1000] }
    )
    .toBe(10);
  await request.dispose();
});

test.afterAll(async () => {
  // The fixture's rack space is small enough that the job may have finished.
  const stats = await api.get<{ job: { status: string } }>(`/api/jobs/${jobId}`);
  if (stats.job.status === 'active') await api.post(`/api/admin/jobs/${jobId}/deactivate`);
  await api.dispose();
});

test('E-13: an opening-rack job offers analysed racks to look up', async ({ page }) => {
  await page.goto(`/jobs/${jobId}`);
  await expect(page.getByText('Analysed racks to try:')).toBeVisible();
  const samples = page.getByTestId('rack-sample');
  await expect(samples).toHaveCount(10);
  // Drawn at random across the job, each once; "Shuffle" draws again.
  expect(new Set(await samples.allInnerTexts()).size).toBe(10);
  const drawn = page.waitForResponse((r) => r.url().includes(`/api/jobs/${jobId}/rack-samples`));
  await page.getByRole('button', { name: 'Shuffle' }).click();
  await drawn;
  await expect(samples).toHaveCount(10);
  const rack = (await samples.first().innerText()).trim();
  await samples.first().click();
  await expect(page.getByLabel('Look up a rack')).toHaveValue(rack);
  const panel = page.locator('.card', { has: page.getByRole('heading', { name: 'Opening racks' }) });
  await expect(panel.locator('tbody tr').first()).toBeVisible();
  // A static player's analyses: the move, its score and equity, and none of
  // a simulation's columns (E-12 shows a simmer's).
  await expect(panel.locator('thead th')).toContainText(['#', 'Move', 'Score', 'Equity']);
  for (const column of ['Win %', 'Iters', 'P1-S', 'P1-BP']) {
    await expect(panel.locator('thead th', { hasText: column })).toHaveCount(0);
  }
});
