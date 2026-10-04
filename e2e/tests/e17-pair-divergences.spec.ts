import { test, expect } from '@playwright/test';
import { AdminApi, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, SEEDED_DATA, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-17: a game-pairs job made through the form keeping only where each pair
 * first diverges shows a signed-in user the two games' positions of one pair
 * on one board: the same turn, the same rack, each player to move in one of
 * them, each player's move drawn where it goes and marked in its list. The fake workers synthesize first divergences the way MAGPIE keeps
 * them: one position per game, the second from the other seat.
 */
let api: AdminApi;
let jobId: string;
let names: [string, string];

test.beforeAll(async ({ browser, playwright }) => {
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  names = [`e17-a-${suffix}`, `e17-b-${suffix}`];
  await api.createStaticConfig(names[0], 'equity');
  await api.createStaticConfig(names[1], 'score');

  const admin = await browser.newContext({ storageState: ADMIN_STATE });
  const form = await admin.newPage();
  await form.goto('/admin/jobs/new');
  await form.getByLabel('Job name').fill(`e17 first divergences ${suffix}`);
  await form.getByLabel('Job type').selectOption({ label: 'Game pairs' });
  const letterdist = form.getByLabel('Letter distribution');
  const fixtureBag = letterdist.locator('option', { hasText: `english_fixture (${SEEDED_DATA},` });
  await letterdist.selectOption((await fixtureBag.getAttribute('value'))!);
  const layout = form.getByLabel('Board layout');
  const board = layout.locator('option', { hasText: `standard15 (${SEEDED_DATA},` });
  await layout.selectOption((await board.getAttribute('value'))!);
  await form.getByLabel('Player 1').selectOption({ label: names[0] });
  await form.getByLabel('Player 2').selectOption({ label: names[1] });
  await form.getByLabel('Pairs per batch').fill('5');
  await form.getByLabel('Pairs to play').fill('30');
  // Offered only once positions are saved.
  const firstDivergence = form.getByLabel('Only where each pair first diverges');
  await expect(firstDivergence).toHaveCount(0);
  await form.getByLabel('Save the positions played').check();
  await firstDivergence.check();
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

test("E-17: a pairs job shows each pair's first divergence as two players' answers on one board", async ({ page }) => {
  await page.goto(`/jobs/${jobId}`);
  await expect(page.getByText('first divergences').first()).toBeVisible();
  await expect(page.getByText('the turn where its two games first diverged')).toBeVisible();

  const pair = page.getByTestId('saved-pair');
  await expect(pair).toBeVisible();
  await expect(pair).toContainText('where the players first chose differently');
  const games = pair.getByTestId('saved-position');
  await expect(games).toHaveCount(2);
  await expect(games.nth(0).getByTestId('position-heading')).toContainText('Game 1 of the pair');
  await expect(games.nth(1).getByTestId('position-heading')).toContainText('Game 2 of the pair');
  // One position from two seats: the same rack to play, a different player
  // to move in each game.
  const rack = (await games.nth(0).getByTestId('position-rack').innerText()).trim();
  await expect(games.nth(1).getByTestId('position-rack')).toHaveText(rack);
  const movers = await Promise.all(
    [0, 1].map(async (i) => (await games.nth(i).getByTestId('position-heading').innerText()).trim())
  );
  expect(movers.some((m) => m.includes(`${names[0]} to move`)), movers.join(' | ')).toBe(true);
  expect(movers.some((m) => m.includes(`${names[1]} to move`)), movers.join(' | ')).toBe(true);
  expect(await games.nth(0).locator('tbody tr').count()).toBeGreaterThan(0);
  expect(await games.nth(1).locator('tbody tr').count()).toBeGreaterThan(0);

  // One board for the two: the same position, with each player's move drawn
  // on it (or one at a time where they share a square), and no CGP text.
  await expect(pair).toHaveAttribute('data-shared-board', '');
  await expect(pair.getByTestId('board')).toHaveCount(1);
  for (const i of [0, 1]) {
    const played = (await games.nth(i).getByTestId('played-move').innerText()).trim();
    expect(played.length).toBeGreaterThan(0);
  }
  await expect(pair.locator('[title="CGP"]')).toHaveCount(0);
  await expect(pair.getByTestId('board-legend')).toContainText('played');

  // The rack finds the pair once, both games of it.
  await page.getByLabel('Rack').fill(rack);
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByText(`Position 1 with the rack ${rack}`)).toBeVisible();
  await expect(pair.getByTestId('saved-position')).toHaveCount(2);
});
