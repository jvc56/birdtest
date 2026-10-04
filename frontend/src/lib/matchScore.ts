/**
 * A games or pairs job's match score, as its table on the job pages shows it:
 * a column per player, and in each row the figure that is better higher --
 * wins, draws, score (a win is a point, a draw half of one) and its share of
 * the games, average score per game, and average spread.
 *
 * Every figure counts games, for a pairs job too: pairs are the SPRT's unit,
 * and their outcomes are the pair-outcome table in the SPRT card. For a job
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

/** A score, W + ½D: "12.5", and a whole number without ".0". */
const points = (n: number) => n.toLocaleString(undefined, { maximumFractionDigits: 1 });
/** Rounded to one place first, so -0.04 reads 0.0 rather than -0.0. */
const tenths = (n: number) => Math.round(n * 10) / 10 || 0;
const signed = (n: number) => `${n > 0 ? '+' : ''}${n.toFixed(1)}`;

/** The games the counts cover: the record's total. */
export function gamesPlayed(g: MatchScoreInput): number {
  return g.wins + g.losses + g.draws;
}

export function matchRows(g: MatchScoreInput): CompareRow[] {
  const games = gamesPlayed(g);
  const scores: [number, number] = [g.wins + g.draws / 2, g.losses + g.draws / 2];
  const pct = (score: number) => (games ? (100 * score) / games : null);
  const pcts: [number | null, number | null] = [pct(scores[0]), pct(scores[1])];
  const means: [number | null, number | null] = [
    g.p1_score_mean === null ? null : tenths(g.p1_score_mean),
    g.p2_score_mean === null ? null : tenths(g.p2_score_mean)
  ];
  const spreads: [number | null, number | null] =
    g.spread_mean === null ? [null, null] : [tenths(g.spread_mean), tenths(-g.spread_mean)];
  const dash = (n: number | null, f: (n: number) => string) => (n === null ? '—' : f(n));
  return [
    { label: 'Wins', values: [g.wins.toLocaleString(), g.losses.toLocaleString()], numbers: [g.wins, g.losses] },
    { label: 'Draws', values: [g.draws.toLocaleString(), g.draws.toLocaleString()], numbers: [g.draws, g.draws] },
    {
      label: 'Score',
      title: 'A win is a point, a draw half of one',
      values: [points(scores[0]), points(scores[1])],
      numbers: scores
    },
    {
      label: 'Score %',
      title: 'The score over the games played',
      values: [dash(pcts[0], (n) => `${n.toFixed(1)}%`), dash(pcts[1], (n) => `${n.toFixed(1)}%`)],
      numbers: pcts
    },
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
