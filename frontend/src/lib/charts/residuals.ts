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
 * The signature of a non-transitive pool: several head-to-heads the scalar
 * ratings get badly wrong at once. One or two can be noise; three is what a
 * rock-paper-scissors triangle produces.
 */
export function isNonTransitive(residuals: RatingResidual[]): boolean {
  return notableResiduals(residuals).length >= NON_TRANSITIVE_MIN;
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
