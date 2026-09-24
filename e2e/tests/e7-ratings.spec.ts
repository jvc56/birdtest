import { test, expect, type Page } from '@playwright/test';
import { AdminApi, seededJob, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-7: the ratings page. An admin creates a pool, adds a config, sees the fit
 * appear, removes it, and sees the ratings change -- the one flow where a
 * write is meant to move numbers elsewhere on the page.
 *
 * The evidence is three finished game-pairs jobs among three static configs:
 * the seeded static-equity vs static-score, and two this journey starts. The
 * fake workers let player 1 win 60% of divergent games, so every job says
 * "player 1 is stronger" by about the same margin -- which no single number
 * per config can satisfy around a triangle. That is what makes a third
 * config's games move the second one's rating. All three jobs are finished
 * and idle before the page is opened, so every refit sees the same games and
 * the numbers can be compared exactly.
 */

const ANCHOR = 'static-equity';
const RATED = 'static-score';

let api: AdminApi;
let third: string;
let poolName: string;

test.beforeAll(async ({ playwright }) => {
  const request = await playwright.request.newContext({ baseURL: env.baseURL });
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  third = `e2e-rated-${suffix}`;
  poolName = `e2e-pool-${suffix}`;
  const thirdId = await api.createStaticConfig(third, 'equity', 5);
  const anchorId = await api.playerConfigId(ANCHOR);
  const ratedId = await api.playerConfigId(RATED);

  const pairs = { job_type: 'game_pairs', pairs_per_batch: 10, min_pairs: 100, max_pairs: 300 };
  const jobs = [
    await api.activeJob({ ...pairs, player1_config_id: ratedId, player2_config_id: thirdId }, 20),
    await api.activeJob({ ...pairs, player1_config_id: anchorId, player2_config_id: thirdId }, 20),
    (await seededJob(request)).id
  ];
  for (const job of jobs) await waitUntilSettled(request, job);
  await request.dispose();

  const data = await api.seededData();
  await api.post('/api/admin/rating-pools', {
    name: poolName,
    variant: 'classic',
    letterdist_id: data.letterdist,
    layout_id: data.layout,
    anchor_player_config_id: anchorId,
    anchor_rating: 1500
  });
});

test.afterAll(async () => {
  await api.dispose();
});

/** A config's row in the pool's table (not in the residuals below it). */
function configRow(page: Page, name: string) {
  return page
    .locator('.card', { has: page.getByRole('heading', { name: 'All configs' }) })
    .locator('tbody tr')
    .filter({ has: page.locator('td:first-child', { hasText: name }) });
}

/** "Last fit <when> (membership) over 160 pairs from 1 job, 33 iterations." */
function fitLine(page: Page) {
  return page.locator('p', { hasText: 'Last fit' });
}

function membershipFit(jobs: string): RegExp {
  // Svelte leaves line breaks inside the sentence, and a regex sees them.
  const words = ['Last fit', '.+', '\\(membership\\) over', '[\\d,]+', `pairs from ${jobs},`, '\\d+', 'iterations\\.'];
  return new RegExp(`^${words.join(' ').replace(/ /g, '\\s+')}$`, 's');
}

/** A config's rating as the table prints it, to one decimal place. */
async function rating(page: Page, name: string): Promise<string> {
  return (await configRow(page, name).locator('td').nth(1).innerText()).trim();
}

async function addMember(page: Page, name: string) {
  await page.getByLabel('Add a player config').selectOption({ label: name });
  await page.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(configRow(page, name)).toHaveCount(1);
}

test('E-7: an admin builds a rating pool and watches membership move the ratings', async ({ page }) => {
  // There is no form for creating a pool yet, so the pool is created over
  // the API (beforeAll) and the journey starts at the ratings list.
  await page.goto('/ratings');
  const listed = page.locator('tbody tr', { hasText: poolName });
  await expect(listed).toContainText('classic · english_fixture · standard15');
  await listed.getByRole('link', { name: poolName }).click();

  await expect(page.getByRole('heading', { name: poolName })).toBeVisible();
  await expect(page.getByText('anchored at 1500')).toBeVisible();
  // A new pool holds only its anchor and has never been fitted.
  await expect(page.getByText('Never computed.')).toBeVisible();

  // Add a config: the pool refits and a rating appears where there was none.
  await addMember(page, RATED);
  await expect(fitLine(page)).toHaveText(membershipFit('1 job'));
  const anchorRow = configRow(page, ANCHOR);
  await expect(anchorRow.locator('td').nth(0)).toContainText('anchor');
  await expect(anchorRow.locator('td').nth(1)).toHaveText('1500.0');
  await expect(anchorRow.locator('td').nth(2)).toHaveText('fixed');
  const alone = await rating(page, RATED);
  expect(alone).toMatch(/^\d+\.\d$/);
  // Player 1 of the seeded job is the anchor, and player 1 wins more often.
  expect(Number(alone)).toBeLessThan(1500);
  await expect(configRow(page, RATED).locator('td').nth(2)).toHaveText(/^±\d+\.\d$/);

  // A third config brings two more jobs' games, and they move the second
  // config's rating even though neither is about it alone.
  await addMember(page, third);
  await expect(fitLine(page)).toHaveText(membershipFit('3 jobs'));
  const withThird = await rating(page, RATED);
  expect(withThird).not.toBe(alone);
  expect(await rating(page, third)).toMatch(/^\d+\.\d$/);

  // Remove it again: the refit takes its games back out, and the rating it
  // moved returns to exactly what those games alone support.
  await configRow(page, third).getByRole('button', { name: 'Remove' }).click();
  await expect(configRow(page, third)).toHaveCount(0);
  await expect(fitLine(page)).toHaveText(membershipFit('1 job'));
  await expect(configRow(page, RATED).locator('td').nth(1)).toHaveText(alone);
  expect(alone).not.toBe(withThird);
});
