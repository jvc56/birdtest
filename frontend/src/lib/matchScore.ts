/**
 * A games or pairs job's match score, as its table on the job pages shows it:
 * a column per player, and a row each for wins, losses (each player's are the
 * other's wins, and fewer is better), draws, average score per game, and
 * average spread.
 *
 * Every figure counts games, for a pairs job too: pairs are the significance test's
 * unit, and their outcomes are the pair-outcome table in the Significance Test card. For a job
 * that runs no test, this is the job's result.
 */
import type { CompareRow } from './compare';

export interface MatchScoreInput {
  /** Player 1's results; player 2's wins are player 1's losses. */
  wins: number;
  losses: number;
  draws: number;
  p1_score_mean: number | null;
  p2_score_mean: number | null;
  /** Player 1's average spread; player 2's is its negation. */
  spread_mean: number | null;
}

/** Rounded to one place first, so -0.04 reads 0.0 rather than -0.0. */
const tenths = (n: number) => Math.round(n * 10) / 10 || 0;
const signed = (n: number) => `${n > 0 ? '+' : ''}${n.toFixed(1)}`;

/** The games the counts cover: the record's total. */
export function gamesPlayed(g: MatchScoreInput): number {
  return g.wins + g.losses + g.draws;
}

export function matchRows(g: MatchScoreInput): CompareRow[] {
  const means: [number | null, number | null] = [
    g.p1_score_mean === null ? null : tenths(g.p1_score_mean),
    g.p2_score_mean === null ? null : tenths(g.p2_score_mean)
  ];
  const spreads: [number | null, number | null] =
    g.spread_mean === null ? [null, null] : [tenths(g.spread_mean), tenths(-g.spread_mean)];
  const dash = (n: number | null, f: (n: number) => string) => (n === null ? '—' : f(n));
  const counts = (label: string, a: number, b: number): CompareRow => ({
    label,
    values: [a.toLocaleString(), b.toLocaleString()],
    numbers: [a, b]
  });
  return [
    counts('Wins', g.wins, g.losses),
    // Each player's losses are the other's wins: fewer is better.
    { ...counts('Losses', g.losses, g.wins), better: 'lower' },
    counts('Draws', g.draws, g.draws),
    {
      label: 'Average score',
      title: 'Points per game',
      values: [dash(means[0], (n) => n.toFixed(1)), dash(means[1], (n) => n.toFixed(1))],
      numbers: means
    },
    {
      label: 'Average spread',
      title: 'Points ahead of the opponent at the end of a game, on average',
      values: [dash(spreads[0], signed), dash(spreads[1], signed)],
      numbers: spreads
    }
  ];
}
