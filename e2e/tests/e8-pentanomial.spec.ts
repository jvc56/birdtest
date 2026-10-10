import { test, expect } from '@playwright/test';
import { seededJob, waitUntilSettled } from '../lib/api';

/**
 * E-8: a game-pairs job's page shows its pentanomial's five pair outcomes as
 * three labelled rows, each read from the player's own side, the match test's
 * verdict in words, and player 1's record in the match score box.
 *
 * The seeded job, once the fake workers have finished it. They favour player
 * 1, so its test must have ended for player 1 -- finding it better, or at the
 * cap -- and never by finding player 2 better. A job the finish check completed shows the
 * verdict it was completed on (`Completed: ...`), not the live status line.
 */
test('E-8: a finished game-pairs job shows its labelled pentanomial and significance-test verdict', async ({
  page,
  request
}) => {
  const job = await seededJob(request);
  await waitUntilSettled(request, job.id);

  await page.goto(`/jobs/${job.id}`);
  await expect(page.getByRole('heading', { level: 1, name: job.name, exact: true })).toBeVisible();
  const card = page.getByTestId('significance-test');
  const verdict = card.locator('p', { hasText: 'Completed:' });
  await expect(verdict).toBeVisible();
  expect((await verdict.innerText()).replace(/\s+/g, ' ').trim()).toMatch(
    /^Completed: (decided: player 1 is better|inconclusive: the job reached its cap first), player 1 at \d+\.\d% to \d+\.\d% after [\d,]+ pairs\. The figures above include the pairs that were in flight then\.$/
  );

  // Player 1's score, with its range, in a sentence; the test
  // explained with the job's own confidence, folded away.
  await expect(card.getByTestId('significance-test-sentence')).toContainText(
    /^static-equity scores \d+\.\d% per game \(95% interval \d+\.\d% to \d+\.\d%\)/
  );
  await expect(card.getByTestId('test-explained')).toContainText('What does the interval mean?');

  // The pair outcomes, a column per player, each row read from the player's
  // own side: player 1's "won both" is bucket 4, player 2's bucket 0.
  const table = card.getByTestId('player-compare');
  await expect(table.locator('thead th')).toHaveText(['Pair outcome', 'static-equity', 'static-score']);
  const rows = table.locator('tbody tr');
  await expect(rows.locator('td:first-child')).toHaveText(['Won both', 'Won one, drew one', 'Even']);

  // Every completed pair is in exactly one bucket, and the shares are of pairs.
  const stats = await (await request.get(`/api/jobs/${job.id}`)).json();
  const pentanomial: number[] = stats.games.pentanomial;
  const pairs: number = stats.games.units_completed;
  expect(pentanomial.reduce((a, b) => a + b, 0)).toBe(pairs);
  // Matched from the start: a marked cell also says "(better)" or "(worse)"
  // to a screen reader.
  const cell = (n: number) =>
    new RegExp(`^${n.toLocaleString('en-US')} \\(${((100 * n) / pairs).toFixed(1)}%\\)`);
  await expect(rows.locator('td:nth-child(2)')).toHaveText([4, 3, 2].map((b) => cell(pentanomial[b])));
  await expect(rows.locator('td:nth-child(3)')).toHaveText([0, 1, 2].map((b) => cell(pentanomial[b])));
  // The higher of a row is marked, the even row never.
  const standing = (n: number, m: number) => (n === m ? 'even' : n > m ? 'better' : 'worse');
  await expect(rows.nth(0).locator('td:nth-child(2)')).toHaveAttribute(
    'data-standing',
    standing(pentanomial[4], pentanomial[0])
  );
  await expect(rows.nth(2).locator('td:nth-child(2)')).toHaveAttribute('data-standing', 'even');
  await expect(card.getByText(`The test runs on all ${pairs.toLocaleString('en-US')} pairs`)).toBeVisible();

  // Player 1's record is the match score's, in a box of its own above the
  // test, a column per player, counted in games; the Significance Test card keeps only the test.
  const score = page.locator('.card', { has: page.getByRole('heading', { name: 'Match score' }) });
  const { wins, losses, draws } = stats.games;
  const scoreRows = score.getByTestId('match-all').getByTestId('player-compare').locator('tbody tr');
  await expect(scoreRows.locator('td:first-child')).toHaveText([
    'Wins', 'Losses', 'Draws', 'Average score', 'Average spread'
  ]);
  const count = (n: number) => new RegExp(`^${n.toLocaleString('en-US')}\\b`);
  await expect(scoreRows.nth(0).locator('td')).toHaveText(['Wins', count(wins), count(losses)]);
  // Each player's losses are the other's wins, and fewer is better.
  await expect(scoreRows.nth(1).locator('td')).toHaveText(['Losses', count(losses), count(wins)]);
  if (wins !== losses) {
    await expect(scoreRows.nth(1).locator('td:nth-child(2)')).toHaveAttribute(
      'data-standing',
      losses < wins ? 'better' : 'worse'
    );
  }
  await expect(scoreRows.nth(2).locator('td')).toHaveText(['Draws', count(draws), count(draws)]);
  await expect(score.getByTestId('match-games')).toContainText(`Over ${(wins + losses + draws).toLocaleString('en-US')} games`);
  // And a second table over the games of the pairs that diverged.
  const divergent = stats.games.divergent;
  const divergentRows = score.getByTestId('match-divergent').getByTestId('player-compare').locator('tbody tr');
  await expect(score.getByTestId('match-divergent')).toContainText('Games that diverged');
  await expect(divergentRows.nth(0).locator('td')).toHaveText(['Wins', count(divergent.wins), count(divergent.losses)]);
  await expect(card.getByText(/^Player 1:/)).toHaveCount(0);
});
