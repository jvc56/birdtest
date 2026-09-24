import { test, expect } from '@playwright/test';
import { seededJob, waitUntilSettled } from '../lib/api';

/**
 * E-8: a game-pairs job's page shows its pentanomial with the five buckets
 * labelled in order, and the SPRT status in words.
 *
 * The seeded job, once the fake workers have finished it. They favour player
 * 1, so its test must have ended for player 1 -- by accepting H1 or at the
 * cap -- and never by accepting H0.
 */
test('E-8: a finished game-pairs job shows its labelled pentanomial and SPRT verdict', async ({
  page,
  request
}) => {
  const job = await seededJob(request);
  await waitUntilSettled(request, job.id);

  await page.goto(`/jobs/${job.id}`);
  await expect(page.getByRole('heading', { name: 'Game pairs' })).toBeVisible();
  const sprt = page.locator('.card', { has: page.getByRole('heading', { name: 'SPRT' }) });
  const verdict = sprt.locator('p', { hasText: '— LLR' });
  await expect(verdict).toBeVisible();
  expect((await verdict.innerText()).replace(/\s+/g, ' ').trim()).toMatch(
    /^(passed \(H1 accepted\)|terminated at max games) — LLR -?\d+\.\d{3} within \[-?\d+\.\d{2}, \d+\.\d{2}\]\. SPRT is not acted on until 100 pairs are complete\.$/
  );

  const table = sprt.locator('table');
  await expect(table.locator('thead th')).toHaveText(['Pair outcome', 'Pairs', 'Share']);
  const rows = table.locator('tbody tr');
  await expect(rows.locator('td:first-child')).toHaveText([
    'P1 lost both',
    'Lost one, drew one',
    'Split 1-1',
    'Won one, drew one',
    'P1 won both'
  ]);

  // Every completed pair is in exactly one bucket, and the shares are of pairs.
  const stats = await (await request.get(`/api/jobs/${job.id}`)).json();
  const pentanomial: number[] = stats.games.pentanomial;
  const pairs: number = stats.games.units_completed;
  expect(pentanomial.reduce((a, b) => a + b, 0)).toBe(pairs);
  await expect(rows.locator('td:nth-child(2)')).toHaveText(pentanomial.map((n) => n.toLocaleString('en-US')));
  await expect(rows.locator('td:nth-child(3)')).toHaveText(
    pentanomial.map((n) => `${((100 * n) / pairs).toFixed(1)}%`)
  );
  await expect(sprt.getByText(`The test runs on all ${pairs.toLocaleString('en-US')} pairs`)).toBeVisible();
});
