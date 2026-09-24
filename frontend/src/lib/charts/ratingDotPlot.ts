/**
 * The arithmetic behind RatingDotPlot.svelte, kept out of the component so it
 * can be tested without rendering: a scale is the kind of thing that, when
 * wrong, draws a plausible picture rather than failing.
 */
import type { RatingRow } from '$lib/api';

export const ROW_HEIGHT = 28;
export const PAD = { top: 8, right: 24, bottom: 28, left: 180 };

/**
 * The widest error bar drawn, in rating points. A config with a handful of
 * pairs can carry a standard error in the thousands (an unmeasurable one is
 * stored as f64::MAX); drawn at full width it would squeeze every other
 * config's interval into a few pixels.
 */
export const MAX_VISIBLE_ERROR = 400;

export interface Bounds {
  lo: number;
  hi: number;
}

/**
 * Unrated configs have no position on the scale — no chain of games links them
 * to the anchor — so they are listed beneath the chart rather than drawn at a
 * number that means nothing.
 */
export function splitByAnchorConnection(ratings: RatingRow[]): {
  rated: RatingRow[];
  unrated: RatingRow[];
} {
  return {
    rated: ratings.filter((r) => r.connected_to_anchor),
    unrated: ratings.filter((r) => !r.connected_to_anchor)
  };
}

/** Clamp runaway intervals so one barely-measured config cannot flatten the
 *  scale for everyone else; the table still reports the real number. */
export function visibleError(row: RatingRow): number {
  return Math.min(row.stderr, MAX_VISIBLE_ERROR);
}

/** The rating range drawn: every visible interval, plus a margin. */
export function dotPlotBounds(rated: RatingRow[]): Bounds {
  if (!rated.length) return { lo: 0, hi: 1 };
  const los = rated.map((r) => r.rating - visibleError(r));
  const his = rated.map((r) => r.rating + visibleError(r));
  const lo = Math.min(...los);
  const hi = Math.max(...his);
  const pad = Math.max(10, (hi - lo) * 0.08);
  return { lo: lo - pad, hi: hi + pad };
}

/** Rating to horizontal pixel position, for a chart `width` pixels wide. */
export function dotPlotX(value: number, bounds: Bounds, width: number): number {
  return PAD.left + ((value - bounds.lo) / (bounds.hi - bounds.lo)) * (width - PAD.left - PAD.right);
}

export function dotPlotHeight(ratedCount: number): number {
  return PAD.top + PAD.bottom + ratedCount * ROW_HEIGHT;
}

/** Vertical centre of the `index`th rated row. */
export function dotPlotRowY(index: number): number {
  return PAD.top + index * ROW_HEIGHT + ROW_HEIGHT / 2;
}

/** Gridline positions: a 1/2/5 × 10^k step giving at most six intervals. */
export function dotPlotTicks(bounds: Bounds): number[] {
  const span = bounds.hi - bounds.lo;
  const step = Math.pow(10, Math.floor(Math.log10(span / 4)));
  const nice = [1, 2, 5, 10].map((m) => m * step).find((s) => span / s <= 6) ?? step;
  const out: number[] = [];
  for (let t = Math.ceil(bounds.lo / nice) * nice; t <= bounds.hi; t += nice) out.push(t);
  return out;
}

export interface DotPlotRow {
  row: RatingRow;
  /** The dot. */
  cx: number;
  cy: number;
  /** The error bar's ends, after clamping. */
  x1: number;
  x2: number;
}

/** Everything the component draws, in pixels. Unrated configs get no position. */
export function layoutDotPlot(
  ratings: RatingRow[],
  width: number
): {
  rated: DotPlotRow[];
  unrated: RatingRow[];
  bounds: Bounds;
  ticks: number[];
  height: number;
} {
  const { rated, unrated } = splitByAnchorConnection(ratings);
  const bounds = dotPlotBounds(rated);
  return {
    rated: rated.map((row, i) => {
      const err = visibleError(row);
      return {
        row,
        cx: dotPlotX(row.rating, bounds, width),
        cy: dotPlotRowY(i),
        x1: dotPlotX(row.rating - err, bounds, width),
        x2: dotPlotX(row.rating + err, bounds, width)
      };
    }),
    unrated,
    bounds,
    ticks: dotPlotTicks(bounds),
    height: dotPlotHeight(rated.length)
  };
}

/** The dot's tooltip. Reports the true standard error, not the clamped one. */
export function dotTitle(row: RatingRow): string {
  return `${row.name}: ${row.rating.toFixed(1)} ± ${row.stderr.toFixed(1)} over ${row.pairs_played.toLocaleString()} pairs`;
}

/** The ratings table's rating column: a dash for a config with no scale. */
export function ratingCell(row: RatingRow): string {
  return row.connected_to_anchor ? row.rating.toFixed(1) : '—';
}

/** The ratings table's ± column: the true, unclamped standard error. */
export function stderrCell(row: RatingRow): string {
  if (row.is_anchor) return 'fixed';
  return row.connected_to_anchor ? `±${row.stderr.toFixed(1)}` : 'unrated';
}
