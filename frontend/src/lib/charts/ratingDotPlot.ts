/**
 * The arithmetic behind RatingDotPlot.svelte, kept out of the component so it
 * can be tested without rendering: a scale is the kind of thing that, when
 * wrong, draws a plausible picture rather than failing.
 */
import type { RatingRow } from '$lib/api';
import { fitLabel, fitLabels } from './labels';

export const ROW_HEIGHT = 28;
/**
 * The margins around the plot at full width: the names on the left, each
 * row's rating and its error on the right.
 */
export const PAD = { top: 8, right: 96, bottom: 28, left: 180 };
/** Below this width the names get a narrower margin, so the plot keeps room on a phone. */
export const NARROW_WIDTH = 520;
const NARROW_LEFT = 108;

/** The margins for a chart `width` pixels wide. */
export function padFor(width: number): typeof PAD {
  return width < NARROW_WIDTH ? { ...PAD, left: NARROW_LEFT } : PAD;
}

/** Standard errors either side of a rating that make its 95% interval. */
export const Z95 = 1.96;

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
 *  scale for everyone else; the table still reports the real number. `k`
 *  standard errors either side: 1 for the thick bar, `Z95` for the thin. */
export function visibleError(row: RatingRow, k = 1): number {
  // The anchor is fixed by definition. Runs before the thirty-second audit
  // stored the fit's own error for it -- f64::MAX with no games -- which,
  // drawn at the cap, stretched the scale; newer runs store 0.
  if (row.is_anchor) return 0;
  return Math.min(k * row.stderr, MAX_VISIBLE_ERROR);
}

/** The rating range drawn: every visible 95% interval, plus a margin. */
export function dotPlotBounds(rated: RatingRow[]): Bounds {
  if (!rated.length) return { lo: 0, hi: 1 };
  const los = rated.map((r) => r.rating - visibleError(r, Z95));
  const his = rated.map((r) => r.rating + visibleError(r, Z95));
  const lo = Math.min(...los);
  const hi = Math.max(...his);
  const pad = Math.max(10, (hi - lo) * 0.08);
  return { lo: lo - pad, hi: hi + pad };
}

/** Rating to horizontal pixel position, for a chart `width` pixels wide. */
export function dotPlotX(value: number, bounds: Bounds, width: number): number {
  const pad = padFor(width);
  return pad.left + ((value - bounds.lo) / (bounds.hi - bounds.lo)) * (width - pad.left - pad.right);
}

export function dotPlotHeight(ratedCount: number): number {
  return PAD.top + PAD.bottom + ratedCount * ROW_HEIGHT;
}

/** Vertical centre of the `index`th rated row. */
export function dotPlotRowY(index: number): number {
  return PAD.top + index * ROW_HEIGHT + ROW_HEIGHT / 2;
}

/**
 * How often a gridline is labelled, for `count` gridlines across a plot
 * `plotWidth` pixels wide: every one where there is room, every second or
 * third on a phone, where four-digit labels ran together.
 */
export function tickLabelStep(count: number, plotWidth: number): number {
  return Math.max(1, Math.ceil((count * 46) / Math.max(plotWidth, 1)));
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
  /** The ±1 standard error bar's ends, after clamping. */
  x1: number;
  x2: number;
  /** The 95% interval's ends, after clamping. */
  w1: number;
  w2: number;
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
  /** Label every this many gridlines. */
  tickStep: number;
  height: number;
  pad: typeof PAD;
} {
  const { rated, unrated } = splitByAnchorConnection(ratings);
  const bounds = dotPlotBounds(rated);
  const pad = padFor(width);
  const ticks = dotPlotTicks(bounds);
  const x = (value: number) => dotPlotX(value, bounds, width);
  return {
    rated: rated.map((row, i) => {
      const err = visibleError(row);
      const wide = visibleError(row, Z95);
      return {
        row,
        cx: x(row.rating),
        cy: dotPlotRowY(i),
        x1: x(row.rating - err),
        x2: x(row.rating + err),
        w1: x(row.rating - wide),
        w2: x(row.rating + wide)
      };
    }),
    unrated,
    bounds,
    ticks,
    tickStep: tickLabelStep(ticks.length, width - pad.left - pad.right),
    height: dotPlotHeight(rated.length),
    pad
  };
}

/** The backend stores an unmeasurable error as f64::MAX. */
function errorText(stderr: number): string {
  return Number.isFinite(stderr) && stderr < 1e300 ? stderr.toFixed(1) : '∞';
}

/** The dot's tooltip. Reports the true standard error, not the clamped one. */
export function dotTitle(row: RatingRow): string {
  const spread = row.is_anchor ? ' (fixed)' : ` ± ${errorText(row.stderr)}`;
  return `${row.name}: ${row.rating.toFixed(1)}${spread} over ${row.pairs_played.toLocaleString()} pairs`;
}

/** The ratings table's rating column: a dash for a config with no scale. */
export function ratingCell(row: RatingRow): string {
  return row.connected_to_anchor ? row.rating.toFixed(1) : '—';
}

/** The ratings table's ± column: the true, unclamped standard error. */
export function stderrCell(row: RatingRow): string {
  if (row.is_anchor) return 'fixed';
  return row.connected_to_anchor ? `±${errorText(row.stderr)}` : 'unrated';
}

/**
 * The most characters a row's name shows, its " (anchor)" included: the left
 * margin is fixed, and a longer name lost its start past the SVG's edge (the
 * audit's pass 25). 20 characters at 12px fit it in lower case and ordinary
 * mixed case; wide capitals can still run past (KL-76).
 */
export const NAME_CHARS = 20;
/** The same in the narrower margin a phone gets (`padFor`). */
export const NARROW_NAME_CHARS = 13;

const ANCHOR = ' (anchor)';

/**
 * Each row's label, told apart from the others drawn (see `fitLabels`), the
 * anchor marked.
 */
export function rowLabels(rows: RatingRow[], chars: number = NAME_CHARS): string[] {
  const labels = fitLabels(rows.map((r) => r.name), chars);
  return labels.map((label, i) =>
    rows[i].is_anchor ? fitLabel(label, chars - ANCHOR.length) + ANCHOR : label
  );
}

/** The rating and its error as the column right of a row prints them: "1641.0", "±12.4" or "fixed". */
export function ratingFigure(row: RatingRow): { rating: string; error: string } {
  return { rating: row.rating.toFixed(1), error: row.is_anchor ? 'fixed' : `±${errorText(row.stderr)}` };
}
