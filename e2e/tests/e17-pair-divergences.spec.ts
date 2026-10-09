import { test, expect } from '@playwright/test';
import { AdminApi, waitUntilSettled } from '../lib/api';
import { ADMIN_STATE, SEEDED_DATA, env } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-17: a game-pairs job made through the form keeping only where each pair
 * first diverges shows a signed-in user the two games' positions of one pair,
 * one player's at a time with a toggle between them: the same turn, the same
 * rack, each player to move in one of them, its move drawn where it goes and
 * marked in its list. The fake workers synthesize first divergences the way MAGPIE keeps
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
  await form.getByLabel('Job Name').fill(`e17 first divergences ${suffix}`);
  await form.getByLabel('Job Type').selectOption({ label: 'Game Pairs' });
  const letterdist = form.getByLabel('Letter Distribution');
  const fixtureBag = letterdist.locator('option', { hasText: `english_fixture (${SEEDED_DATA},` });
  await letterdist.selectOption((await fixtureBag.getAttribute('value'))!);
  const layout = form.getByLabel('Board', { exact: true });
  const board = layout.locator('option', { hasText: `standard15 (${SEEDED_DATA},` });
  await layout.selectOption((await board.getAttribute('value'))!);
  await form.getByLabel('Player 1').selectOption({ label: names[0] });
  await form.getByLabel('Player 2').selectOption({ label: names[1] });
  await form.getByLabel('Pairs Per Task').fill('5');
  await form.getByLabel('Pairs To Play').fill('30');
  // Offered only once positions are saved.
  const firstDivergence = form.getByLabel('Only Where Each Pair First Diverges');
  await expect(firstDivergence).toHaveCount(0);
  await form.getByLabel('Position Recorder').check();
  await firstDivergence.check();
  await form.getByRole('button', { name: 'Create job' }).click();
  await expect(form).toHaveURL(/\/admin\/jobs\/[0-9a-f-]{36}$/);
  jobId = form.url().split('/').pop()!;
  await admin.close();
  await api.allocate(jobId, 10);

  const request = await playwright.request.newContext({ baseURL: env.baseURL });
  await waitUntilSettled(request, jobId);
  await request.dispose();
});

test.afterAll(async () => {
  await api.dispose();
});

test("E-17: a pairs job shows each pair's first divergence, one player's answer at a time", async ({ page }) => {
  await page.goto(`/jobs/${jobId}`);
  await expect(page.getByText('first divergences').first()).toBeVisible();
  await expect(page.getByText('the turn where its two games first diverged')).toBeVisible();

  const pair = page.getByTestId('saved-pair');
  await expect(pair).toBeVisible();
  await expect(pair).toContainText('where the players first chose differently');
  // One player's answer at a time: one board, its move drawn on it, its
  // ranked moves; a toggle switches to the other game's.
  const shown = pair.getByTestId('saved-position');
  const toggles = pair.getByTestId('pair-toggle');
  await expect(shown).toHaveCount(1);
  await expect(toggles).toHaveCount(2);
  await expect(pair.getByTestId('board')).toHaveCount(1);
  await expect(toggles.nth(0)).toHaveAttribute('aria-pressed', 'true');
  const read = async () => ({
    heading: (await shown.getByTestId('position-heading').innerText()).trim(),
    rack: (await shown.getByTestId('position-rack').innerText()).trim(),
    played: (await shown.getByTestId('played-move').innerText()).trim(),
    moves: await shown.getByTestId('position-moves').locator('tbody tr').count()
  });
  const first = await read();
  expect(first.heading).toContain('Game 1 of the pair');
  await toggles.nth(1).click();
  await expect(toggles.nth(1)).toHaveAttribute('aria-pressed', 'true');
  await expect(shown.getByTestId('position-heading')).toContainText('Game 2 of the pair');
  const second = await read();
  await expect(pair.getByTestId('board')).toHaveCount(1);
  // One position from two seats: the same rack to play, a different player
  // to move in each game, each with its own move played and list.
  expect(second.rack).toBe(first.rack);
  const movers = [first.heading, second.heading];
  expect(movers.some((m) => m.includes(`${names[0]} to move`)), movers.join(' | ')).toBe(true);
  expect(movers.some((m) => m.includes(`${names[1]} to move`)), movers.join(' | ')).toBe(true);
  expect(first.played.length && second.played.length).toBeGreaterThan(0);
  expect(first.moves).toBeGreaterThan(0);
  expect(second.moves).toBeGreaterThan(0);
  await expect(pair.locator('[title="CGP"]')).toHaveCount(0);
  const rack = first.rack;

  // The rack finds the pair once, both games of it.
  await page.getByLabel('Rack').fill(rack);
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page.getByText(`Position 1 with the rack ${rack}`)).toBeVisible();
  await expect(pair.getByTestId('pair-toggle')).toHaveCount(2);
});
