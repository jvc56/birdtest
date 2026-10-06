import { test, expect, devices } from '@playwright/test';
import { AdminApi, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, SEEDED_DATA, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-12: a games job made through the form with "Position Recorder"
 * ticked shows a signed-in user one saved position at a time on its board --
 * a random one, or one of a rack's -- and tells a signed-out visitor to sign
 * in. The fake workers play synthetic games, 18 to 26 turns each, whose
 * positions are real boards: tiles, both racks, and the play before.
 */
let api: AdminApi;
let jobId: string;

test.beforeAll(async ({ browser, playwright }) => {
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  const [a, b] = [`e12-a-${suffix}`, `e12-b-${suffix}`];
  // Two simmers, so every position -- either player's turn -- was simulated:
  // the fake workers shape a position as its mover's config would, as MAGPIE
  // does, and a static player's carries no simulation statistics.
  await api.createSimConfig(a);
  await api.createSimConfig(b);

  const admin = await browser.newContext({ storageState: ADMIN_STATE });
  const form = await admin.newPage();
  await form.goto('/admin/jobs/new');
  await form.getByLabel('Job Name').fill(`e12 saved positions ${suffix}`);
  await form.getByLabel('Job Type').selectOption({ label: 'Games' });
  const letterdist = form.getByLabel('Letter Distribution');
  const fixtureBag = letterdist.locator('option', { hasText: `english_fixture (${SEEDED_DATA},` });
  await letterdist.selectOption((await fixtureBag.getAttribute('value'))!);
  const layout = form.getByLabel('Board', { exact: true });
  const board = layout.locator('option', { hasText: `standard15 (${SEEDED_DATA},` });
  await layout.selectOption((await board.getAttribute('value'))!);
  await form.getByLabel('Player 1').selectOption({ label: a });
  await form.getByLabel('Player 2').selectOption({ label: b });
  await form.getByLabel('Games Per Task').fill('2');
  // No match test, the form's default: the job plays its four games and stops.
  await expect(form.getByLabel('Significance Test')).not.toBeChecked();
  await expect(form.getByLabel('Minimum Games')).toHaveCount(0);
  await form.getByLabel('Games To Play').fill('4');
  await form.getByLabel('Position Recorder').check();
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

test('E-12: a signed-in user draws saved positions at random and searches them by rack', async ({ page, browser }) => {
  const signedOut = await browser.newContext({ storageState: { cookies: [], origins: [] } });
  const visitor = await signedOut.newPage();
  await visitor.goto(`/jobs/${jobId}`);
  await expect(visitor.getByText('Sign in to search them')).toBeVisible();
  await signedOut.close();

  await page.goto(`/jobs/${jobId}`);
  await expect(page.getByRole('heading', { name: 'Saved positions', exact: true })).toBeVisible();
  const shown = page.getByTestId('saved-position');
  const board = page.getByTestId('board');
  await expect(shown).toBeVisible();

  // A position after a play, rather than an opening turn's empty board or
  // one after an exchange -- and not the second turn's either, whose only
  // tiles are the opening play's: tiles are down, and only the play's are
  // outlined.
  await expect(async () => {
    const outlined = await board.locator('.tile.last').count();
    const tiles = await board.locator('.tile').count();
    if (!outlined || tiles <= outlined) {
      await page.getByRole('button', { name: 'Random position' }).click();
    }
    expect(outlined).toBeGreaterThan(0);
    expect(tiles).toBeGreaterThan(outlined);
  }).toPass({ timeout: 30_000 });
  await expect(board.locator('.tile.last').first()).toBeVisible();
  await expect(shown).toContainText('after');
  // Both racks and scores, the player to move marked, and the ranked moves.
  await expect(shown.getByTestId('rack')).toHaveCount(2);
  await expect(shown.getByTestId('score')).toHaveCount(2);
  await expect(shown.getByText('to move', { exact: true })).toHaveCount(1);
  const moves = shown.getByTestId('position-moves');
  expect(await moves.locator('tbody tr').count()).toBeGreaterThan(0);
  // Both players simulate: each move's win percentage and its first two
  // plies' statistics, the reply's first.
  await expect(moves.locator('thead th')).toContainText(['Win %', 'P1-S', 'P1-BP', 'P2-S', 'P2-BP']);
  // The move played from here: named, marked in the list, and -- unless it
  // was a pass or an exchange -- drawn where it goes, apart from the tiles
  // already down. No CGP text under the board.
  const played = (await shown.getByTestId('played-move').innerText()).trim();
  expect(played.length).toBeGreaterThan(0);
  await expect(shown.locator('tr[data-played]')).toHaveCount(1);
  if (!/^(pass|\(exch )/.test(played)) {
    expect(await board.locator('.played').count()).toBeGreaterThan(0);
  }
  await expect(shown.locator('[title="CGP"]')).toHaveCount(0);

  // Search for the rack on the board, typed in lower case and backwards:
  // what comes back holds it.
  const rack = (await shown.getByTestId('position-rack').innerText()).trim();
  await page.getByLabel('Rack').fill([...rack.toLowerCase()].reverse().join(''));
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByText(`Position 1 with the rack ${rack}`)).toBeVisible();
  await expect(shown.getByTestId('position-rack')).toHaveText(rack);
  await expect(board).toBeVisible();
  const searching = page.getByTestId('rack-search');
  const next = searching.getByRole('button', { name: 'Next', exact: true });
  if (await next.isVisible()) {
    await next.click();
    await expect(page.getByText(`Position 2 with the rack ${rack}`)).toBeVisible();
    await expect(shown.getByTestId('position-rack')).toHaveText(rack);
    await searching.getByRole('button', { name: 'Previous', exact: true }).click();
    await expect(page.getByText(`Position 1 with the rack ${rack}`)).toBeVisible();
  }

  await page.getByLabel('Rack').fill('QQQQQQQ');
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByText('No saved position has the rack QQQQQQQ.')).toBeVisible();
  await expect(shown).toHaveCount(0);
});

test('E-12b: the board fits a phone', async ({ browser }) => {
  const phone = await browser.newContext({ ...devices['Pixel 5'], storageState: ADMIN_STATE });
  const page = await phone.newPage();
  await page.goto(`/jobs/${jobId}`);
  await expect(page.getByTestId('board')).toBeVisible();
  const screen = page.viewportSize()!.width;
  expect(screen).toBeLessThan(400);
  // The board shrinks to the screen rather than pushing the page sideways.
  const box = (await page.getByTestId('board').boundingBox())!;
  expect(box.x + box.width).toBeLessThanOrEqual(screen);
  const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(scrollWidth, 'page is wider than the screen').toBeLessThanOrEqual(screen);
  await phone.close();
});
