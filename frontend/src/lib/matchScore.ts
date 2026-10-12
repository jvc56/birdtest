/**
 * A games or pairs job's match score, as its card on the job pages shows it:
 * each player's share of the points, the games each won and the draws on a bar
 * between them, each player's average score, and the average spread.
 *
 * Every figure counts games, for a pairs job too: pairs are the significance test's
 * unit, and their outcomes are the pair-outcome table in the Significance Test card. For a job
 * that runs no test, this is the job's result.
 */

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

export interface MatchScore {
  /** The games the counts cover: the record's total. */
  played: number;
  /** The games each player won, player 1 first: player 2's are player 1's losses. */
  wins: [number, number];
  draws: number;
  /** Each player's share of the points, a draw counting half, as "57.73%"; null before any game. */
  shares: [string, string] | null;
  /** The player ahead on points, 0 when level. */
  leader: 0 | 1 | 2;
  /** Each player's average score per game, as "431.20", or "—" before any game. */
  averages: [string, string];
  /**
   * The average spread as the points the player ahead leads by, "19.70";
   * `ahead` is 0 when it rounds to nothing. Null before any game.
   */
  spread: { points: string; ahead: 0 | 1 | 2 } | null;
}

/** Rounded to two places first, so -0.004 reads 0.00 rather than -0.00. */
const hundredths = (n: number) => Math.round(n * 100) / 100 || 0;
const twoPlaces = (n: number | null) => (n === null ? '—' : hundredths(n).toFixed(2));

/** The games the counts cover: the record's total. */
export function gamesPlayed(g: MatchScoreInput): number {
  return g.wins + g.losses + g.draws;
}

export function matchScore(g: MatchScoreInput): MatchScore {
  const played = gamesPlayed(g);
  const share = played === 0 ? null : (g.wins + g.draws / 2) / played;
  const spread = g.spread_mean === null ? null : hundredths(g.spread_mean);
  return {
    played,
    wins: [g.wins, g.losses],
    draws: g.draws,
    shares: share === null ? null : [`${(share * 100).toFixed(2)}%`, `${((1 - share) * 100).toFixed(2)}%`],
    leader: g.wins > g.losses ? 1 : g.losses > g.wins ? 2 : 0,
    averages: [twoPlaces(g.p1_score_mean), twoPlaces(g.p2_score_mean)],
    spread:
      spread === null ? null : { points: Math.abs(spread).toFixed(2), ahead: spread > 0 ? 1 : spread < 0 ? 2 : 0 }
  };
}
