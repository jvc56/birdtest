/**
 * The arithmetic behind ResidualMatrix.svelte: actual minus predicted score
 * per head-to-head, which of them are worth attention, and how each is drawn.
 */
import type { RatingResidual } from '$lib/api';

/** Anything at or past this is a disagreement worth a reader's attention. */
export const NOTABLE = 0.05;
/** The residual that fills a bar's half-width and saturates its colour. */
export const SCALE_MAX = 0.25;
/** How many notable head-to-heads it takes to call the pool non-transitive. */
export const NON_TRANSITIVE_MIN = 3;
/**
 * How many standard errors a residual must be from zero to count toward that
 * call. Without it a young pool -- a handful of pairs per head-to-head, and a
 * fit whose prior pulls lightly observed configs toward the anchor -- showed
 * the banner on sampling noise alone.
 */
export const NON_TRANSITIVE_Z = 3;

export function residual(cell: RatingResidual): number {
  return cell.actual - cell.predicted;
}

/**
 * Largest disagreement first, the order the page promises. The API already
 * returns them so (`ORDER BY abs(actual - predicted) DESC, row, col`); sorting
 * here as well keeps the promise independent of the source. Ties fall back to
 * the same row, col order. Returns a new array.
 */
export function sortResiduals(residuals: RatingResidual[]): RatingResidual[] {
  return [...residuals].sort(
    (a, b) =>
      Math.abs(residual(b)) - Math.abs(residual(a)) ||
      (a.row < b.row ? -1 : a.row > b.row ? 1 : 0) ||
      (a.col < b.col ? -1 : a.col > b.col ? 1 : 0)
  );
}

export function isNotable(delta: number): boolean {
  return Math.abs(delta) >= NOTABLE;
}

export function notableResiduals(residuals: RatingResidual[]): RatingResidual[] {
  return residuals.filter((cell) => isNotable(residual(cell)));
}

/**
 * A residual in standard errors of the observed score. A pair's score varies
 * by at most what a single game's would, so `p(1 - p) / pairs` bounds the
 * variance of the mean; `p` is the predicted score, kept off 0 and 1 so a
 * lopsided prediction does not make every miss infinitely significant.
 */
export function zScore(cell: RatingResidual): number {
  if (cell.pairs <= 0) return 0;
  const p = Math.min(Math.max(cell.predicted, 0.01), 0.99);
  return residual(cell) / Math.sqrt((p * (1 - p)) / cell.pairs);
}

/** Notable, and too far from zero to be sampling noise. */
export function significantResiduals(residuals: RatingResidual[]): RatingResidual[] {
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
export function isNonTransitive(residuals: RatingResidual[]): boolean {
  return significantResiduals(residuals).length >= NON_TRANSITIVE_MIN;
}

/** |delta| as a fraction of the scale, capped at 1. */
export function magnitude(delta: number): number {
  return Math.min(Math.abs(delta) / SCALE_MAX, 1);
}

/** Warm for "scored above prediction", cool for below, grey at zero. */
export function residualFill(delta: number): string {
  const t = magnitude(delta);
  if (t < 0.04) return 'hsl(217 19% 20%)';
  const [h, s] = delta > 0 ? [25, 85] : [205, 85];
  return `hsl(${h} ${s}% ${20 + t * 32}%)`;
}

/**
 * The bar, in percent of its track: it grows right from the centre for a
 * positive residual and left for a negative one.
 */
export function residualBar(delta: number): { left: number; width: number } {
  const half = magnitude(delta) * 50;
  return { left: delta > 0 ? 50 : 50 - half, width: half };
}

/** In percentage points, signed: "+7.5", "-3.0", "0.0". */
export function formatResidual(delta: number): string {
  return `${delta > 0 ? '+' : ''}${(100 * delta).toFixed(1)}`;
}
