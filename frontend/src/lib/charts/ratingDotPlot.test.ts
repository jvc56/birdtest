import { describe, expect, it } from 'vitest';
import type { RatingRow } from '$lib/api';
import {
  dotPlotBounds,
  dotPlotTicks,
  dotPlotX,
  dotTitle,
  layoutDotPlot,
  MAX_VISIBLE_ERROR,
  PAD,
  ratingCell,
  ROW_HEIGHT,
  stderrCell,
  visibleError
} from './ratingDotPlot';

function row(
  id: string,
  rating: number,
  stderr: number,
  extra: Partial<RatingRow> = {}
): RatingRow {
  return {
    player_config_id: id,
    name: id.toUpperCase(),
    rating,
    stderr,
    pairs_played: 1000,
    connected_to_anchor: true,
    is_anchor: false,
    ...extra
  };
}

const WIDTH = 720;
const INNER = WIDTH - PAD.left - PAD.right; // 516 px of plot

const anchor = row('a', 1500, 0, { is_anchor: true });
// One standard error above the anchor: its bar's lower end sits on the anchor.
const oneSeAbove = row('c', 1540, 40);
const other = row('b', 1600, 50);

describe('F-CHART-1 RatingDotPlot positions', () => {
  // Intervals span [1500, 1650]; the 8% margin (12) is above the 10 floor.
  const layout = layoutDotPlot([anchor, other, oneSeAbove], WIDTH);
  const pxPerPoint = INNER / 174;
  const at = (id: string) => layout.rated.find((r) => r.row.player_config_id === id)!;

  it('computes the domain from every interval plus a margin', () => {
    expect(layout.bounds.lo).toBeCloseTo(1488, 9);
    expect(layout.bounds.hi).toBeCloseTo(1662, 9);
  });

  it("places the anchor's dot at the anchor rating", () => {
    expect(at('a').cx).toBeCloseTo(PAD.left + 12 * pxPerPoint, 9);
    expect(at('a').cx).toBeCloseTo(dotPlotX(1500, layout.bounds, WIDTH), 9);
    // Stderr 0: the anchor's bar is a point on its dot.
    expect(at('a').x1).toBeCloseTo(at('a').cx, 9);
    expect(at('a').x2).toBeCloseTo(at('a').cx, 9);
  });

  it('places a config one standard error away at the expected offset', () => {
    expect(at('c').cx - at('a').cx).toBeCloseTo(40 * pxPerPoint, 9);
    // Its interval's lower end lands exactly on the anchor's dot.
    expect(at('c').x1).toBeCloseTo(at('a').cx, 9);
    // And a bar is ±1 SE, symmetric about the dot.
    expect(at('b').cx - at('b').x1).toBeCloseTo(50 * pxPerPoint, 9);
    expect(at('b').x2 - at('b').cx).toBeCloseTo(50 * pxPerPoint, 9);
  });

  it('maps the domain ends to the plot edges', () => {
    expect(dotPlotX(layout.bounds.lo, layout.bounds, WIDTH)).toBeCloseTo(PAD.left, 9);
    expect(dotPlotX(layout.bounds.hi, layout.bounds, WIDTH)).toBeCloseTo(WIDTH - PAD.right, 9);
  });

  it('stacks rows in input order, one ROW_HEIGHT apart, and sizes the chart to them', () => {
    expect(layout.rated.map((r) => r.row.player_config_id)).toEqual(['a', 'b', 'c']);
    expect(layout.rated.map((r) => r.cy)).toEqual([
      PAD.top + ROW_HEIGHT / 2,
      PAD.top + ROW_HEIGHT * 1.5,
      PAD.top + ROW_HEIGHT * 2.5
    ]);
    expect(layout.height).toBe(PAD.top + PAD.bottom + 3 * ROW_HEIGHT);
  });

  it('pads a single anchor-only pool by the 10-point floor', () => {
    expect(dotPlotBounds([anchor])).toEqual({ lo: 1490, hi: 1510 });
  });

  it('draws gridlines on a nice step, inside the domain', () => {
    const ticks = layout.ticks;
    expect(ticks.length).toBeGreaterThanOrEqual(2);
    expect(ticks.length).toBeLessThanOrEqual(7);
    for (const t of ticks) {
      expect(t).toBeGreaterThanOrEqual(layout.bounds.lo);
      expect(t).toBeLessThanOrEqual(layout.bounds.hi);
    }
    const step = ticks[1] - ticks[0];
    expect([10, 20, 50, 100]).toContain(Math.round(step));
    ticks.forEach((t, i) => i && expect(t - ticks[i - 1]).toBeCloseTo(step, 9));
    expect(dotPlotTicks({ lo: 1000, hi: 2000 })).toEqual([1000, 1200, 1400, 1600, 1800, 2000]);
  });
});

describe('F-CHART-2 runaway error bars', () => {
  const barely = row('d', 1520, 5000, { pairs_played: 3 });

  it('clamps the drawn interval', () => {
    expect(visibleError(barely)).toBe(MAX_VISIBLE_ERROR);
    expect(visibleError(other)).toBe(50);
    const layout = layoutDotPlot([anchor, other, barely], WIDTH);
    const d = layout.rated.find((r) => r.row.player_config_id === 'd')!;
    const pxPerPoint = INNER / (layout.bounds.hi - layout.bounds.lo);
    expect(d.x2 - d.x1).toBeCloseTo(2 * MAX_VISIBLE_ERROR * pxPerPoint, 9);
    // The domain is set by the clamped bar, not the true one.
    expect(layout.bounds.lo).toBeCloseTo(1520 - 400 - 64, 9);
    expect(layout.bounds.hi).toBeCloseTo(1520 + 400 + 64, 9);
  });

  it('does not let one barely-measured config flatten the scale', () => {
    const without = layoutDotPlot([anchor, other], WIDTH);
    const withIt = layoutDotPlot([anchor, other, barely], WIDTH);
    const barWidth = (l: typeof without) => {
      const b = l.rated.find((r) => r.row.player_config_id === 'b')!;
      return b.x2 - b.x1;
    };
    // Unclamped, the domain would be ~11,600 points wide and b's ±50 bar
    // about 4 px; clamped it keeps a readable width.
    expect(barWidth(withIt)).toBeGreaterThan(25);
    expect(barWidth(withIt) / barWidth(without)).toBeGreaterThan(0.15);
  });

  it('handles an unmeasurable config (stored as f64::MAX) with finite positions', () => {
    const unmeasured = row('u', 1500, Number.MAX_VALUE, { pairs_played: 0 });
    const layout = layoutDotPlot([anchor, unmeasured], WIDTH);
    for (const r of layout.rated) {
      expect(Number.isFinite(r.cx)).toBe(true);
      expect(Number.isFinite(r.x1)).toBe(true);
      expect(Number.isFinite(r.x2)).toBe(true);
    }
    expect(layout.ticks.every(Number.isFinite)).toBe(true);
  });

  it('still reports the true standard error in the table and the tooltip', () => {
    expect(stderrCell(barely)).toBe('±5000.0');
    expect(dotTitle(barely)).toBe('D: 1520.0 ± 5000.0 over 3 pairs');
    expect(ratingCell(barely)).toBe('1520.0');
  });

  it('labels the anchor as fixed rather than with a standard error', () => {
    expect(stderrCell(anchor)).toBe('fixed');
    expect(stderrCell(other)).toBe('±50.0');
  });
});

describe('F-CHART-3 configs not connected to the anchor', () => {
  const island = row('z', 9000, 3, { connected_to_anchor: false });

  it('are listed as unrated and not drawn at a position', () => {
    const layout = layoutDotPlot([anchor, island, other], WIDTH);
    expect(layout.rated.map((r) => r.row.player_config_id)).toEqual(['a', 'b']);
    expect(layout.unrated).toEqual([island]);
    expect(layout.height).toBe(PAD.top + PAD.bottom + 2 * ROW_HEIGHT);
  });

  it('do not stretch the scale', () => {
    expect(layoutDotPlot([anchor, island, other], WIDTH).bounds).toEqual(
      layoutDotPlot([anchor, other], WIDTH).bounds
    );
  });

  it('show no number in the table', () => {
    expect(ratingCell(island)).toBe('—');
    expect(stderrCell(island)).toBe('unrated');
  });

  it('a pool with nothing rated has an empty plot and a harmless domain', () => {
    const layout = layoutDotPlot([island], WIDTH);
    expect(layout.rated).toEqual([]);
    expect(layout.unrated).toEqual([island]);
    expect(layout.bounds).toEqual({ lo: 0, hi: 1 });
  });
});
