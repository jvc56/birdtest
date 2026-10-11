/**
 * The residuals behind the cross table's hover and its warning: actual minus
 * predicted score per head-to-head, which of them are worth attention, and
 * when they add up to a pool no single rating per config can describe.
 *
 * Every function here takes each head-to-head **once**. The API serves the
 * cross table from both sides, and a mirrored cell is the same miss with its
 * sign flipped; pass `oneSide` of it (`$lib/ratingPool`).
 */
import type { RatingHeadToHead } from '$lib/api';

/** Anything at or past this is a disagreement worth a reader's attention. */
export const NOTABLE = 0.05;
/** How many notable head-to-heads it takes to call the pool non-transitive. */
export const NON_TRANSITIVE_MIN = 3;
/**
 * How many standard errors a residual must be from zero to count toward that
 * call. Without it a young pool -- a handful of pairs per head-to-head --
 * showed the banner on sampling noise alone.
 */
export const NON_TRANSITIVE_Z = 3;

export function residual(cell: RatingHeadToHead): number {
  return cell.actual - cell.predicted;
}

export function isNotable(delta: number): boolean {
  return Math.abs(delta) >= NOTABLE;
}

export function notableResiduals(residuals: RatingHeadToHead[]): RatingHeadToHead[] {
  return residuals.filter((cell) => isNotable(residual(cell)));
}

/**
 * A residual in standard errors of the observed score. A pair's score varies
 * by at most what a single game's would, so `p(1 - p) / pairs` bounds the
 * variance of the mean; `p` is the predicted score, kept off 0 and 1 so a
 * lopsided prediction does not make every miss infinitely significant. The
 * bound rather than the cell's own `stderr`, which a handful of pairs that
 * all went one way puts at 0.
 */
export function zScore(cell: RatingHeadToHead): number {
  if (cell.pairs <= 0) return 0;
  const p = Math.min(Math.max(cell.predicted, 0.01), 0.99);
  return residual(cell) / Math.sqrt((p * (1 - p)) / cell.pairs);
}

/** Notable, and too far from zero to be sampling noise. */
export function significantResiduals(residuals: RatingHeadToHead[]): RatingHeadToHead[] {
  return notableResiduals(residuals).filter(
    (cell) => Math.abs(zScore(cell)) >= NON_TRANSITIVE_Z
  );
}

/**
 * The signature of a non-transitive pool: several head-to-heads the scalar
 * ratings get badly wrong at once, each on enough pairs that it is not
 * noise. One or two can be; three is what a rock-paper-scissors triangle
 * produces.
 */
export function isNonTransitive(residuals: RatingHeadToHead[]): boolean {
  return significantResiduals(residuals).length >= NON_TRANSITIVE_MIN;
}

/** In percentage points, signed: "+7.5", "-3.0", "0.0". */
export function formatResidual(delta: number): string {
  return `${delta > 0 ? '+' : ''}${(100 * delta).toFixed(1)}`;
}
