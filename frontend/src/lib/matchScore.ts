/**
 * A games or pairs job's match score, as its box on the job pages shows it:
 * player 1's record, its score -- a win is a point, a draw half of one -- over
 * every game played, and the players' average scores and spread.
 *
 * Every figure counts games, for a pairs job too: pairs are the SPRT's unit,
 * and their outcomes are the pentanomial in the SPRT card. For a job that runs
 * no test, this is the job's result.
 */

export interface MatchScoreInput {
  wins: number;
  losses: number;
  draws: number;
  p1_score_mean: number | null;
  p2_score_mean: number | null;
  spread_mean: number | null;
}

export interface MatchScore {
  /** Player 1's wins–losses–draws, "12–8–1". */
  record: string;
  /** Every game played: the record's total. */
  games: number;
  /** W + ½D, "12.5"; a whole number has no ".0". */
  score: string;
  /** The score's share of the games, "59.5%"; null before any game. */
  scorePct: string | null;
  /** Average points per game, one decimal place; null before any game. */
  p1Mean: string | null;
  p2Mean: string | null;
  /** Player 1's average spread, signed ("+13.4", "-2.5", "0.0"). */
  spread: string | null;
}

const oneDecimal = (value: number | null) => (value === null ? null : value.toFixed(1));

export function matchScore(g: MatchScoreInput): MatchScore {
  const games = g.wins + g.losses + g.draws;
  const score = g.wins + g.draws / 2;
  // Rounded to one place first, so a spread of -0.04 reads 0.0, not -0.0.
  const spread = g.spread_mean === null ? null : Math.round(g.spread_mean * 10) / 10;
  return {
    record: [g.wins, g.losses, g.draws].map((n) => n.toLocaleString()).join('–'),
    games,
    score: score.toLocaleString(undefined, { maximumFractionDigits: 1 }),
    scorePct: games ? `${((100 * score) / games).toFixed(1)}%` : null,
    p1Mean: oneDecimal(g.p1_score_mean),
    p2Mean: oneDecimal(g.p2_score_mean),
    spread: spread === null ? null : `${spread > 0 ? '+' : ''}${(spread || 0).toFixed(1)}`
  };
}
