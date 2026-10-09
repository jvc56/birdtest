import { test, expect, type Page } from '@playwright/test';
import { AdminApi, seededJob, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-7: the ratings page. An admin creates a pool through its form, adds a config, sees the fit
 * appear, removes it, and sees the ratings change -- the one flow where a
 * write is meant to move numbers elsewhere on the page. Then moves the anchor,
 * and deletes the pool.
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
let letterdistId: string;
let layoutId: string;

test.beforeAll(async ({ playwright }) => {
  const request = await playwright.request.newContext({ baseURL: env.baseURL });
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  third = `e2e-rated-${suffix}`;
  poolName = `e2e-pool-${suffix}`;
  const thirdId = await api.createStaticConfig(third, 'equity', 5);
  const anchorId = await api.playerConfigId(ANCHOR);
  const ratedId = await api.playerConfigId(RATED);

  const pairs = {
    job_type: 'game_pairs',
    pairs_per_batch: 10,
    test_enabled: true,
    min_pairs: 100,
    max_pairs: 300
  };
  const jobs = [
    await api.activeJob({ ...pairs, player_config_ids: [ratedId, thirdId] }, 20),
    await api.activeJob({ ...pairs, player_config_ids: [anchorId, thirdId] }, 20),
    (await seededJob(request)).id
  ];
  for (const job of jobs) await waitUntilSettled(request, job);
  await request.dispose();

  const data = await api.seededData();
  letterdistId = data.letterdist;
  layoutId = data.layout;
});

test.afterAll(async () => {
  await api.dispose();
});

/** A config's row in the pool's table (not in the cross table below it). */
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
  await page.goto('/ratings');
  await page.getByRole('link', { name: 'New rating pool' }).click();
  await page.getByLabel('Pool name').fill(poolName);
  await page.getByLabel('Variant').selectOption('classic');
  await page.getByLabel('Letter distribution').selectOption(letterdistId);
  await page.getByLabel('Board layout').selectOption(layoutId);
  await page.getByLabel('Anchor player config').selectOption({ label: ANCHOR });
  await page.getByLabel('Anchor rating').fill('1500');
  await page.getByRole('button', { name: 'Create rating pool' }).click();
  // Created with only its anchor: the rest are added on the pool's page, below.
  await expect(page.getByRole('heading', { name: poolName })).toBeVisible();

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

  // The cross table: three configs, each against the other two from its own
  // side -- a win % with its error over the average spread -- and its rating
  // last, as the table above prints it.
  const cross = page.getByTestId('cross-table');
  await expect(cross.locator('tbody tr')).toHaveCount(3);
  const cells = cross.locator('td[title]');
  await expect(cells).toHaveCount(6);
  await expect(cells.first()).toHaveText(/^\s*\d+\.\d% ±\d+\.\d\s*[-+]?\d+\.\d\s*$/);
  await expect(cells.first()).toHaveAttribute('title', / against .+ The ratings predict /);
  const ratedCross = cross.locator('tbody tr', { has: page.locator('th', { hasText: RATED }) });
  await expect(ratedCross.locator('td').last()).toHaveText(withThird);

  // Remove it again: the refit takes its games back out, and the rating it
  // moved returns to exactly what those games alone support.
  await configRow(page, third).getByRole('button', { name: 'Remove' }).click();
  await expect(configRow(page, third)).toHaveCount(0);
  await expect(fitLine(page)).toHaveText(membershipFit('1 job'));
  await expect(configRow(page, RATED).locator('td').nth(1)).toHaveText(alone);
  expect(alone).not.toBe(withThird);

  // Move the anchor to the other member, pinned at 1600: the pool refits on
  // the new scale, and the old anchor sits as far from it as before, the other
  // way (to the table's rounding).
  await page.getByLabel('Anchor player config').selectOption({ label: RATED });
  await page.getByLabel('Anchor rating').fill('1600');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(fitLine(page)).toContainText('(anchor)');
  await expect(page.getByText('anchored at 1600')).toBeVisible();
  const ratedRow = configRow(page, RATED);
  await expect(ratedRow.locator('td').nth(0)).toContainText('anchor');
  await expect(ratedRow.locator('td').nth(1)).toHaveText('1600.0');
  await expect(ratedRow.locator('td').nth(2)).toHaveText('fixed');
  const oldAnchor = Number(await rating(page, ANCHOR));
  expect(Math.abs(oldAnchor - (1600 + 1500 - Number(alone)))).toBeLessThan(0.2);

  // Delete it: confirmed, then back on the list, where it is gone.
  page.once('dialog', (dialog) => dialog.accept());
  await page.getByRole('button', { name: 'Delete pool' }).click();
  await expect(page).toHaveURL(/\/ratings$/);
  await expect(page.locator('tbody tr', { hasText: poolName })).toHaveCount(0);
});
