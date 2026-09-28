import { test, expect } from '@playwright/test';
import { AdminApi, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-12: a games job that saves its positions lets a signed-in user page
 * through them and search them by rack, and tells a signed-out visitor to
 * sign in. The fake workers synthesise the captured positions, 18 to 26 a
 * game, so four games are plenty for two pages.
 */
let api: AdminApi;
let jobId: string;

test.beforeAll(async ({ playwright }) => {
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  jobId = await api.activeJob(
    {
      name: `e12 saved positions ${suffix}`,
      job_type: 'games',
      player1_config_id: await api.createStaticConfig(`e12-a-${suffix}`, 'equity'),
      player2_config_id: await api.createStaticConfig(`e12-b-${suffix}`, 'score'),
      games_per_batch: 2,
      min_games: 4,
      max_games: 4,
      capture_positions: true
    },
    10
  );
  const request = await playwright.request.newContext({ baseURL: env.baseURL });
  await waitUntilSettled(request, jobId);
  await request.dispose();
});

test.afterAll(async () => {
  await api.dispose();
});

test('E-12: a signed-in user pages through and searches a job\'s saved positions', async ({ page, browser }) => {
  const signedOut = await browser.newContext({ storageState: { cookies: [], origins: [] } });
  const visitor = await signedOut.newPage();
  await visitor.goto(`/jobs/${jobId}`);
  await expect(visitor.getByText('Sign in to search them')).toBeVisible();
  await signedOut.close();

  await page.goto(`/jobs/${jobId}`);
  await expect(page.getByRole('heading', { name: 'Saved positions', exact: true })).toBeVisible();
  const entries = page.locator('p', { hasText: 'moves ranked' });
  await expect(entries).toHaveCount(10);
  await page.getByRole('button', { name: 'Load more' }).click();
  await expect(entries).toHaveCount(20);

  // Search for the rack the newest position was played from, typed in lower
  // case: every result holds it.
  const rack = (await entries.first().locator('span.font-mono').first().innerText()).trim();
  await page.getByLabel('Rack').fill(rack.toLowerCase());
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByRole('button', { name: 'Show all' })).toBeVisible();
  const found = await entries.count();
  expect(found).toBeGreaterThan(0);
  for (let i = 0; i < found; i++) {
    await expect(entries.nth(i).locator('span.font-mono').first()).toHaveText(rack);
  }
});
