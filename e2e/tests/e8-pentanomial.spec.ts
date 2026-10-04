import { test, expect } from '@playwright/test';
import { seededJob, waitUntilSettled } from '../lib/api';

/**
 * E-8: a game-pairs job's page shows its pentanomial with the five buckets
 * labelled in order, the SPRT status in words, and player 1's record in the
 * match score box.
 *
 * The seeded job, once the fake workers have finished it. They favour player
 * 1, so its test must have ended for player 1 -- by accepting H1 or at the
 * cap -- and never by accepting H0. A job the finish check completed shows the
 * verdict it was completed on (`Completed: ...`), not the live status line.
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
  const verdict = sprt.locator('p', { hasText: 'Completed:' });
  await expect(verdict).toBeVisible();
  expect((await verdict.innerText()).replace(/\s+/g, ' ').trim()).toMatch(
    /^Completed: (passed \(H1 accepted\)|stopped at its cap), LLR -?\d+\.\d{3} after [\d,]+ pairs\. With the pairs that were in flight then, LLR -?\d+\.\d{3}, bounds \[-?\d+\.\d{2}, -?\d+\.\d{2}\]\.$/
  );

  // The test explained with the job's own numbers, folded away.
  await expect(sprt.getByTestId('sprt-explained')).toContainText('What do the LLR and bounds mean?');

  // The pair outcomes, a column per player, each row read from the player's
  // own side: player 1's "won both" is bucket 4, player 2's bucket 0.
  const table = sprt.getByTestId('player-compare');
  await expect(table.locator('thead th')).toHaveText(['Pair outcome', 'static-equity', 'static-score']);
  const rows = table.locator('tbody tr');
  await expect(rows.locator('td:first-child')).toHaveText(['Won both', 'Won one, drew one', 'Even']);

  // Every completed pair is in exactly one bucket, and the shares are of pairs.
  const stats = await (await request.get(`/api/jobs/${job.id}`)).json();
  const pentanomial: number[] = stats.games.pentanomial;
  const pairs: number = stats.games.units_completed;
  expect(pentanomial.reduce((a, b) => a + b, 0)).toBe(pairs);
  // Matched from the start: a marked cell also says "(higher)" or "(lower)"
  // to a screen reader.
  const cell = (n: number) =>
    new RegExp(`^${n.toLocaleString('en-US')} \\(${((100 * n) / pairs).toFixed(1)}%\\)`);
  await expect(rows.locator('td:nth-child(2)')).toHaveText([4, 3, 2].map((b) => cell(pentanomial[b])));
  await expect(rows.locator('td:nth-child(3)')).toHaveText([0, 1, 2].map((b) => cell(pentanomial[b])));
  // The higher of a row is marked, the even row never.
  const standing = (n: number, m: number) => (n === m ? 'even' : n > m ? 'higher' : 'lower');
  await expect(rows.nth(0).locator('td:nth-child(2)')).toHaveAttribute(
    'data-standing',
    standing(pentanomial[4], pentanomial[0])
  );
  await expect(rows.nth(2).locator('td:nth-child(2)')).toHaveAttribute('data-standing', 'even');
  await expect(sprt.getByText(`The test runs on all ${pairs.toLocaleString('en-US')} pairs`)).toBeVisible();

  // Player 1's record is the match score's, in a box of its own above the
  // test, a column per player, counted in games; the SPRT card keeps only the test.
  const score = page.locator('.card', { has: page.getByRole('heading', { name: 'Match score' }) });
  const { wins, losses, draws } = stats.games;
  const scoreRows = score.getByTestId('player-compare').locator('tbody tr');
  await expect(scoreRows.nth(0).locator('td')).toHaveText([
    'Wins',
    new RegExp(`^${wins.toLocaleString('en-US')}\\b`),
    new RegExp(`^${losses.toLocaleString('en-US')}\\b`)
  ]);
  await expect(scoreRows.nth(1).locator('td')).toHaveText(['Draws', ...[draws, draws].map((n: number) => n.toLocaleString('en-US'))]);
  await expect(score.getByTestId('match-games')).toContainText(`Over ${(wins + losses + draws).toLocaleString('en-US')} games`);
  await expect(sprt.getByText(/^Player 1:/)).toHaveCount(0);
});
