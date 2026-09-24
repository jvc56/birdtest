/**
 * The job page's pentanomial table: a game-pairs job's five pair outcomes.
 *
 * Bucket `i` holds the pairs in which player 1 scored `i` half-points across
 * the pair's two games (see `GameStats.pentanomial`): 0 is lost both, 4 is won
 * both. An off-by-one in the labels would invert the reading of every paired
 * job, so the mapping lives here, where it is tested.
 */

export const PENTANOMIAL_LABELS = [
  'P1 lost both',
  'Lost one, drew one',
  'Split 1-1',
  'Won one, drew one',
  'P1 won both'
] as const;

/**
 * A bucket's share of all pairs, as the table prints it ("12.5"). The
 * denominator is pairs — every pair lands in exactly one bucket — never games,
 * which would halve every share. No pairs yet reads 0.0, not NaN.
 */
export function pairShare(count: number, pairs: number): string {
  return pairs ? ((100 * count) / pairs).toFixed(1) : '0.0';
}

export interface PentanomialRow {
  label: (typeof PENTANOMIAL_LABELS)[number];
  pairs: number;
  /** Percent of all pairs, formatted to one decimal place. */
  share: string;
}

/** One row per bucket, in bucket order. `pairs` is the job's completed pairs. */
export function pentanomialRows(
  pentanomial: readonly [number, number, number, number, number],
  pairs: number
): PentanomialRow[] {
  return PENTANOMIAL_LABELS.map((label, i) => ({
    label,
    pairs: pentanomial[i],
    share: pairShare(pentanomial[i], pairs)
  }));
}
