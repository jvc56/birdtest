import { test, expect } from '@playwright/test';
import { seededJob, waitUntilSettled } from '../lib/api';

/**
 * E-8: a game-pairs job's page shows its pentanomial's five pair outcomes as
 * one bar's key, each part named for the player it favours, the significance
 * test's verdict, player 1's score and the pairs played as tiles, and player
 * 1's record in the match score box.
 *
 * The seeded job, once the fake workers have finished it. They favour player
 * 1, so its test must have ended for player 1 -- finding it better, or at the
 * cap -- and never by finding player 2 better. A job the finish check completed shows the
 * verdict it was completed on and when, not the live status.
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
  const stats = await (await request.get(`/api/jobs/${job.id}`)).json();
  const pairs: number = stats.games.units_completed;
  const pct = (x: number) => `${(x * 100).toFixed(2)}%`;

  // The verdict: player 1 found better, by name and when, or the cap reached
  // first -- never player 2.
  const decided = stats.games.decided;
  expect(['player1_better', 'inconclusive']).toContain(decided.status);
  if (decided.status === 'player1_better') {
    await expect(card.getByTestId('test-chip')).toHaveText('static-equity better');
    await expect(card.getByTestId('test-verdict')).toContainText('static-equity');
    await expect(card.getByTestId('test-verdict')).toContainText(
      `is better, decided at ${decided.units.toLocaleString('en-US')} pairs`
    );
  } else {
    await expect(card.getByTestId('test-chip')).toHaveText('inconclusive');
    await expect(card.getByTestId('test-verdict')).toContainText('Neither better');
  }

  // Player 1's score and its interval to two places; the pairs played against
  // the cap; the test explained with the job's own confidence, folded away.
  const result = stats.games.test;
  await expect(card.getByTestId('test-score')).toContainText(pct(result.mean));
  await expect(card.getByTestId('test-score')).toContainText(
    `95% interval ${pct(result.lower)} to ${pct(result.upper)}`
  );
  await expect(card.getByTestId('test-progress')).toContainText(pairs.toLocaleString('en-US'));
  await expect(card.getByTestId('test-explained')).toContainText('What does the interval mean?');

  // The pair outcomes, player 1's best first: each read from the side it
  // favours, so player 1's "won both" is bucket 4 and player 2's bucket 0.
  const pentanomial: number[] = stats.games.pentanomial;
  expect(pentanomial.reduce((a, b) => a + b, 0)).toBe(pairs);
  const outcomes = card.getByTestId('pair-outcomes');
  await expect(outcomes.locator('dt')).toHaveText([
    'static-equity won both',
    'static-equity won one, drew one',
    'Even',
    'static-score won one, drew one',
    'static-score won both'
  ]);
  for (const bucket of [4, 3, 2, 1, 0]) {
    await expect(outcomes.getByTestId(`pair-outcome-${bucket}`).locator('dd')).toHaveText([
      pentanomial[bucket].toLocaleString('en-US'),
      `${((100 * pentanomial[bucket]) / pairs).toFixed(1)}%`
    ]);
  }

  // Player 1's record is the match score's, in a box of its own above the
  // test, counted in games; the Significance Test card keeps only the test.
  const score = page.getByTestId('match-score');
  const { wins, losses, draws } = stats.games;
  const fmt = (n: number) => n.toLocaleString('en-US');
  // As the card works them out, so the last place rounds the same way.
  const p1Share = (wins + draws / 2) / (wins + losses + draws);
  const record = (w: number, l: number, d: number) =>
    `static-equity won ${fmt(w)}, ${fmt(d)} draw${d === 1 ? '' : 's'}, static-score won ${fmt(l)}`;
  // Each player's share of the points, a draw half, to two places, and the
  // games each won on the bar: player 2's wins are player 1's losses.
  await expect(score.getByTestId('match-share')).toHaveText([
    `${(p1Share * 100).toFixed(2)}%`,
    `${((1 - p1Share) * 100).toFixed(2)}%`
  ]);
  await expect(score.getByTestId('match-bar')).toHaveAttribute('aria-label', record(wins, losses, draws));
  await expect(score.getByTestId('match-games')).toContainText(`Over ${fmt(wins + losses + draws)} games`);
  // And a switch to the games of the pairs that diverged.
  const divergent = stats.games.divergent;
  await score.getByTestId('match-scope-divergent').click();
  await expect(score.getByTestId('match-scope-divergent')).toHaveAttribute('aria-pressed', 'true');
  await expect(score.getByTestId('match-bar')).toHaveAttribute(
    'aria-label',
    record(divergent.wins, divergent.losses, divergent.draws)
  );
  await expect(score.getByTestId('match-games')).toContainText('whose games did not play identically');
  await expect(card.getByText(/^Player 1:/)).toHaveCount(0);
});
