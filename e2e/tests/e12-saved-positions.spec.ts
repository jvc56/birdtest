import { test, expect } from '@playwright/test';
import { AdminApi, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, SEEDED_DATA, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-12: a games job made through the form with "Save the positions played"
 * ticked lets a signed-in user page through them and search them by rack, and
 * tells a signed-out visitor to sign in. The fake workers synthesise the
 * captured positions, 18 to 26 a game, so four games are plenty for two pages.
 */
let api: AdminApi;
let jobId: string;

test.beforeAll(async ({ browser, playwright }) => {
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  const [a, b] = [`e12-a-${suffix}`, `e12-b-${suffix}`];
  await api.createStaticConfig(a, 'equity');
  await api.createStaticConfig(b, 'score');

  const admin = await browser.newContext({ storageState: ADMIN_STATE });
  const form = await admin.newPage();
  await form.goto('/admin/jobs/new');
  await form.getByLabel('Job name').fill(`e12 saved positions ${suffix}`);
  await form.getByLabel('Job type').selectOption({ label: 'Games' });
  const letterdist = form.getByLabel('Letter distribution');
  const fixtureBag = letterdist.locator('option', { hasText: `english_fixture (${SEEDED_DATA},` });
  await letterdist.selectOption((await fixtureBag.getAttribute('value'))!);
  const layout = form.getByLabel('Board layout');
  const board = layout.locator('option', { hasText: `standard15 (${SEEDED_DATA},` });
  await layout.selectOption((await board.getAttribute('value'))!);
  await form.getByLabel('Player 1').selectOption({ label: a });
  await form.getByLabel('Player 2').selectOption({ label: b });
  await form.getByLabel('Games per batch').fill('2');
  // No SPRT, the form's default: the job plays its four games and stops.
  await expect(form.getByLabel('Run an SPRT')).not.toBeChecked();
  await expect(form.getByLabel('Min before SPRT')).toHaveCount(0);
  await form.getByLabel('Games to play').fill('4');
  await form.getByLabel('Save the positions played').check();
  await form.getByRole('button', { name: 'Create job' }).click();
  await expect(form).toHaveURL(/\/admin\/jobs\/[0-9a-f-]{36}$/);
  jobId = form.url().split('/').pop()!;
  await admin.close();
  await api.post(`/api/admin/jobs/${jobId}/activate`, { allocation: 10 });

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
