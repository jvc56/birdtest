/**
 * The SPRT card's pair-outcome table: a game-pairs job's five pair outcomes,
 * with a column per player.
 *
 * Bucket `i` holds the pairs in which player 1 scored `i` half-points across
 * the pair's two games (see `GameStats.pentanomial`): 0 is lost both, 4 is won
 * both. Player 2's outcome is the mirror -- player 1's "lost both" is player
 * 2's "won both" -- so three rows, each read from the player's own side, hold
 * all five buckets: won both, won one and drew one, and even (a 1-1 split or
 * two draws), which the players share. In every row more is better for the
 * player who has it. An off-by-one here would invert the reading of every
 * paired job, so the mapping lives here, where it is tested.
 */
import type { CompareRow } from '$lib/compare';

/**
 * A bucket's share of all pairs, as the table prints it ("12.5"). The
 * denominator is pairs — every pair lands in exactly one bucket — never games,
 * which would halve every share. No pairs yet reads 0.0, not NaN.
 */
export function pairShare(count: number, pairs: number): string {
  return pairs ? ((100 * count) / pairs).toFixed(1) : '0.0';
}

/** The rows, best outcome first, and each player's bucket in them: [player 1's, player 2's]. */
export const PAIR_OUTCOMES: { label: string; title: string; buckets: [number, number] }[] = [
  { label: 'Won both', title: 'Won both games of the pair', buckets: [4, 0] },
  { label: 'Won one, drew one', title: 'Won one game of the pair and drew the other', buckets: [3, 1] },
  {
    label: 'Even',
    title: 'Won one game and lost the other, or drew both: a 1-1 pair, the same for both players',
    buckets: [2, 2]
  }
];

/** One row per outcome. `pairs` is the job's completed pairs. */
export function pentanomialRows(
  pentanomial: readonly [number, number, number, number, number],
  pairs: number
): CompareRow[] {
  return PAIR_OUTCOMES.map(({ label, title, buckets }) => {
    const counts = buckets.map((b) => pentanomial[b]) as [number, number];
    return {
      label,
      title,
      values: counts.map((n) => `${n.toLocaleString()} (${pairShare(n, pairs)}%)`) as [string, string],
      numbers: counts
    };
  });
}
