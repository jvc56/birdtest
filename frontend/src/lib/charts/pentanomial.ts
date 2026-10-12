/**
 * The Significance Test card's pair outcomes: a game-pairs job's five pair
 * outcomes as the parts of one bar, from player 1 winning both games of a pair
 * to player 2 winning both.
 *
 * Bucket `i` holds the pairs in which player 1 scored `i` half-points across
 * the pair's two games (see `GameStats.pentanomial`): 0 is lost both, 4 is won
 * both. Player 2's outcome is the mirror -- player 1's "lost both" is player
 * 2's "won both" -- so the bar runs from bucket 4 to bucket 0, each part named
 * for the player it favours, and even pairs (a 1-1 split or two draws) in the
 * middle. An off-by-one here would invert the reading of every paired job, so
 * the mapping lives here, where it is tested.
 */

/**
 * A bucket's share of all pairs, as the key prints it ("12.5"). The
 * denominator is pairs — every pair lands in exactly one bucket — never games,
 * which would halve every share. No pairs yet reads 0.0, not NaN.
 */
export function pairShare(count: number, pairs: number): string {
  return pairs ? ((100 * count) / pairs).toFixed(1) : '0.0';
}

export interface PairOutcome {
  /** The pentanomial bucket. */
  bucket: number;
  label: string;
  /** The player the outcome favours, 0 or 1; null for even. */
  player: 0 | 1 | null;
  /** Both games of the pair, rather than one and a draw. */
  both: boolean;
  count: number;
  share: string;
}

/** The bar's parts in order, player 1's best outcome first. `pairs` is the job's completed pairs. */
export function pairOutcomes(
  pentanomial: readonly [number, number, number, number, number],
  pairs: number,
  names: [string, string]
): PairOutcome[] {
  const part = (bucket: number, label: string, player: 0 | 1 | null, both: boolean): PairOutcome => ({
    bucket,
    label,
    player,
    both,
    count: pentanomial[bucket],
    share: pairShare(pentanomial[bucket], pairs)
  });
  return [
    part(4, `${names[0]} won both`, 0, true),
    part(3, `${names[0]} won one, drew one`, 0, false),
    part(2, 'Even', null, false),
    part(1, `${names[1]} won one, drew one`, 1, false),
    part(0, `${names[1]} won both`, 1, true)
  ];
}
