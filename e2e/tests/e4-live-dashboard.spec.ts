import { test, expect, type Page } from '@playwright/test';
import { ADMIN_STATE, SEEDED_DATA } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/** A static player through the form, with the wordmap and rack info table
 *  turned off: nothing below tier 6 builds either, and a job waiting on one
 *  would never dispatch. */
async function createStaticConfig(page: Page, name: string, sort: 'equity' | 'score') {
  await page.goto('/admin/player-configs/new');
  await page.getByLabel('Name').fill(name);
  await page.getByLabel('Recorder type (-r)').selectOption('best');
  await expect(page.getByLabel('Lexicon (-l)')).toHaveValue(/.+/);
  await page.getByLabel('Sort strategy (-s)').selectOption(sort);
  await page.getByRole('button', { name: 'Show advanced options' }).click();
  await page.getByLabel('Use wordmap (-w)').uncheck();
  await page.getByLabel('Use rack info table (-rit)').uncheck();
  await page.getByRole('button', { name: 'Create' }).click();
  await expect(page).toHaveURL(/\/admin\/player-configs$/);
  await expect(page.locator('tbody tr', { hasText: name })).toContainText(sort);
}

/** "123 / 40,000" beside the progress bar, as a number. */
async function pairsCompleted(page: Page): Promise<number> {
  const text = await page
    .locator('div.mb-1', { hasText: 'pairs completed' })
    .locator('span.tabular-nums')
    .innerText();
  return Number(text.split('/')[0].replace(/[^0-9]/g, ''));
}

/**
 * E-4: an admin creates two player configs and a game-pairs job, activates it
 * with an allocation, and watches its dashboard move over SSE as the fake
 * workers contribute. The one journey where the built app, the stream, the
 * scheduler and a worker are all in play at once.
 */
test('E-4: an admin creates configs and a job, activates it, and watches it fill live', async ({ page }) => {
  const suffix = crypto.randomUUID().slice(0, 8);
  const p1 = `e2e-equity-${suffix}`;
  const p2 = `e2e-score-${suffix}`;
  await createStaticConfig(page, p1, 'equity');
  await createStaticConfig(page, p2, 'score');

  await page.goto('/admin/jobs/new');
  const jobName = `e2e pairs ${suffix}`;
  await page.getByLabel('Job name').fill(jobName);
  await page.getByLabel('Job type').selectOption({ label: 'Game pairs' });
  // Neither the bag nor the board is chosen for the admin.
  const letterdist = page.getByLabel('Letter distribution');
  const layout = page.getByLabel('Board layout');
  await expect(letterdist).toHaveValue('');
  await expect(layout).toHaveValue('');
  const fixtureBag = letterdist.locator('option', { hasText: `english_fixture (${SEEDED_DATA},` });
  await letterdist.selectOption((await fixtureBag.getAttribute('value'))!);
  const board = layout.locator('option', { hasText: `standard15 (${SEEDED_DATA},` });
  await layout.selectOption((await board.getAttribute('value'))!);
  await page.getByLabel('Player 1').selectOption({ label: p1 });
  await page.getByLabel('Player 2').selectOption({ label: p2 });
  // One pair per task, so results arrive steadily rather than in lumps.
  await page.getByLabel('Pairs per batch').fill('1');
  // The test is off unless asked for; on, so the page has one to watch too.
  await page.getByLabel('Run an SPRT').check();
  await expect(page.getByLabel('Min before SPRT')).toBeVisible();
  await page.getByRole('button', { name: 'Create job' }).click();

  await expect(page).toHaveURL(/\/admin\/jobs\/[0-9a-f-]{36}$/);
  const jobId = page.url().split('/').pop()!;
  const header = page.locator('main header');
  // Titled by the name it was given, its type beside it; its status in the
  // figures below.
  await expect(header.getByRole('heading', { name: jobName })).toBeVisible();
  await expect(header.getByText('Game pairs', { exact: true })).toBeVisible();
  const status = page.getByTestId('job-status');
  await expect(status.getByText('inactive', { exact: true })).toBeVisible();

  // Activating refetches the job once, after the action; wait that out.
  const isJobFetch = (url: string, method: string) =>
    method === 'GET' && new URL(url).pathname === `/api/jobs/${jobId}`;
  const refetched = page.waitForResponse((r) => isJobFetch(r.url(), r.request().method()));
  await page.getByLabel('Allocation %').fill('20');
  await page.getByRole('button', { name: 'Activate', exact: true }).click();
  await refetched;
  await expect(page.getByText('Job activated.')).toBeVisible();
  await expect(status.getByText('active', { exact: true })).toBeVisible();

  // From here on the page must update itself. It fetches the job over REST
  // only on load and after an admin action, so count those fetches: the
  // numbers moving while none happens is the stream at work.
  const refetches: string[] = [];
  page.on('request', (req) => {
    if (isJobFetch(req.url(), req.method())) refetches.push(req.url());
  });

  const first = await pairsCompleted(page);
  await expect.poll(() => pairsCompleted(page), { timeout: 60_000 }).toBeGreaterThan(first);
  const second = await pairsCompleted(page);
  await expect.poll(() => pairsCompleted(page), { timeout: 60_000 }).toBeGreaterThan(second);
  expect(refetches).toEqual([]);

  // The rest of the dashboard moves with it.
  // The admin page has the public page's match score and SPRT boxes, where it
  // had a one-line summary.
  const sprt = page.locator('.card', { has: page.getByRole('heading', { name: 'SPRT' }) });
  await expect(
    sprt.getByText(/^(running — LLR|Completed: passed \(H1 accepted\), LLR) -?\d+\.\d{3}/)
  ).toBeVisible();
  const score = page.locator('.card', { has: page.getByRole('heading', { name: 'Match score' }) });
  await expect(score.getByTestId('match-record')).toHaveText(/^\d[\d,]*–\d[\d,]*–\d[\d,]*$/);
  await expect(score.getByText(/^Average score:/)).toBeVisible();
  await expect(page.getByText('No contributions yet.')).toHaveCount(0);
  await expect(
    page.locator('.card', { has: page.getByRole('heading', { name: 'Contributors' }) }).getByText(/^Anonymous · /).first()
  ).toBeVisible();

  // And the admin takes the job out of rotation again, handing its share back.
  await page.getByRole('button', { name: 'Deactivate' }).click();
  await expect(page.getByText('Job deactivated.')).toBeVisible();
  await expect(status.getByText('inactive', { exact: true })).toBeVisible();
});
