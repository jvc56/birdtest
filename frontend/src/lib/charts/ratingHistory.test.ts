import { describe, expect, it } from 'vitest';
import type { RatingHistoryPoint } from '$lib/api';
import {
  assignColors,
  groupHistory,
  HEIGHT,
  historyDomain,
  historyX,
  historyY,
  PAD,
  PALETTE,
  preferredSlot,
  SERIES_CAP,
  seriesPath
} from './ratingHistory';

const DAY = 86_400_000;
const T0 = Date.parse('2026-01-01T00:00:00Z');

function point(id: string, day: number, rating: number): RatingHistoryPoint {
  return {
    computed_at: new Date(T0 + day * DAY).toISOString(),
    player_config_id: id,
    name: `name-${id}`,
    rating,
    stderr: 10
  };
}

/** A history of `fits` runs where config i's rating follows `ratingAt(i, fit)`. */
function history(ids: string[], fits: number, ratingAt: (i: number, fit: number) => number) {
  const out: RatingHistoryPoint[] = [];
  for (let fit = 0; fit < fits; fit++) {
    ids.forEach((id, i) => out.push(point(id, fit, ratingAt(i, fit))));
  }
  return out;
}

const colorOf = (series: { id: string; color: string }[]) =>
  new Map(series.map((s) => [s.id, s.color]));

describe('F-CHART-4 RatingHistoryChart series cap', () => {
  it('caps at six series and reports how many it omitted', () => {
    const ids = Array.from({ length: 9 }, (_, i) => `cfg-${i}`);
    const grouped = groupHistory(history(ids, 3, (i) => 1500 + i));
    expect(SERIES_CAP).toBe(6);
    expect(grouped.series).toHaveLength(6);
    expect(grouped.hidden).toBe(3);
  });

  it('picks the six by latest rating, not peak or first rating', () => {
    // cfg-fall led for two fits and then collapsed; cfg-rise started last and
    // ends on top. Only the latest fit decides.
    const h = [
      ...history(['c1', 'c2', 'c3', 'c4', 'c5', 'c6'], 3, (i) => 1600 + i * 10),
      point('fall', 0, 2000),
      point('fall', 1, 1990),
      point('fall', 2, 1000),
      point('rise', 0, 900),
      point('rise', 1, 1500),
      point('rise', 2, 2100)
    ];
    const grouped = groupHistory(h);
    const shown = grouped.series.map((s) => s.id);
    expect(shown).toEqual(['rise', 'c6', 'c5', 'c4', 'c3', 'c2']);
    expect(shown).not.toContain('fall');
    expect(grouped.hidden).toBe(2);
  });

  it('uses the latest point even if the points arrive out of order', () => {
    const h = [point('a', 2, 1400), point('b', 0, 1450), point('a', 0, 1600), point('b', 2, 1500)];
    const grouped = groupHistory(h);
    expect(grouped.series.map((s) => s.id)).toEqual(['b', 'a']);
    expect(grouped.series[1].points.map((p) => p.rating)).toEqual([1600, 1400]);
  });

  it('omits nothing, and says so, at or below the cap', () => {
    const ids = Array.from({ length: 6 }, (_, i) => `cfg-${i}`);
    expect(groupHistory(history(ids, 2, (i) => 1500 + i)).hidden).toBe(0);
    expect(groupHistory([]).series).toEqual([]);
    expect(groupHistory([]).hidden).toBe(0);
  });

  it("keeps one series per config with every point, named by the config", () => {
    const grouped = groupHistory(history(['x', 'y'], 4, (i, fit) => 1500 + i * 50 + fit));
    const y = grouped.series[0];
    expect(y.id).toBe('y');
    expect(y.name).toBe('name-y');
    expect(y.points).toEqual([0, 1, 2, 3].map((fit) => ({ t: T0 + fit * DAY, rating: 1550 + fit })));
  });
});

describe('F-CHART-5 colour by config identity', () => {
  const six = ['cfg-01', 'cfg-02', 'cfg-03', 'cfg-04', 'cfg-05', 'cfg-06'];

  it('gives every drawn series a distinct palette colour, even when ids clash', () => {
    // Two pairs here hash to the same slot; the colours must still differ.
    const slots = six.map(preferredSlot);
    expect(new Set(slots).size).toBeLessThan(six.length);
    const colors = [...assignColors(six).values()];
    expect(new Set(colors).size).toBe(6);
    for (const c of colors) expect(PALETTE).toContain(c);
  });

  it('does not repaint a config when ratings cross between fits', () => {
    // Fit 0: a > b > c. Fit 1: c > b > a. The ranking inverts; colours don't.
    const early = [point('a', 0, 1600), point('b', 0, 1550), point('c', 0, 1500)];
    const late = [...early, point('a', 1, 1500), point('b', 1, 1550), point('c', 1, 1600)];
    const before = groupHistory(early);
    const after = groupHistory(late);
    expect(before.series.map((s) => s.id)).toEqual(['a', 'b', 'c']);
    expect(after.series.map((s) => s.id)).toEqual(['c', 'b', 'a']);
    expect(colorOf(after.series)).toEqual(colorOf(before.series));
  });

  it('filtering the list does not repaint the survivors', () => {
    const full = history(six, 2, (i) => 1500 + i * 10);
    const all = colorOf(groupHistory(full).series);
    const survivors = ['cfg-01', 'cfg-03', 'cfg-06'];
    const filtered = colorOf(
      groupHistory(full.filter((p) => survivors.includes(p.player_config_id))).series
    );
    for (const id of survivors) expect(filtered.get(id)).toBe(all.get(id));
  });

  it('a config that holds its own slot keeps it under every filtering', () => {
    // Exhaustively: every subset of up to six of these eight ids.
    const pool = ['cfg-01', 'cfg-02', 'cfg-03', 'cfg-04', 'cfg-05', 'cfg-06', 'cfg-07', 'cfg-08'];
    const own = (id: string) => PALETTE[preferredSlot(id)];
    for (let mask = 1; mask < 1 << pool.length; mask++) {
      const subset = pool.filter((_, i) => mask & (1 << i));
      if (subset.length > SERIES_CAP) continue;
      const colors = assignColors(subset);
      expect(new Set(colors.values()).size).toBe(subset.length);
      for (let sub = mask; sub; sub = (sub - 1) & mask) {
        const smaller = pool.filter((_, i) => sub & (1 << i));
        const smallerColors = assignColors(smaller);
        for (const id of smaller) {
          if (colors.get(id) === own(id)) expect(smallerColors.get(id)).toBe(own(id));
        }
      }
    }
  });

  it('is independent of input order', () => {
    const shuffled = ['cfg-05', 'cfg-02', 'cfg-06', 'cfg-01', 'cfg-04', 'cfg-03'];
    expect(assignColors(shuffled)).toEqual(assignColors(six));
  });
});

describe('RatingHistoryChart scales', () => {
  it('needs two points before there is anything to draw', () => {
    expect(historyDomain([])).toBeNull();
    expect(historyDomain(groupHistory([point('a', 0, 1500)]).series)).toBeNull();
    expect(historyDomain(groupHistory([point('a', 0, 1500), point('a', 1, 1520)]).series)).toEqual({
      t0: T0,
      t1: T0 + DAY,
      r0: 1495,
      r1: 1525
    });
  });

  it('maps time and rating onto the plot area', () => {
    const d = { t0: T0, t1: T0 + 10 * DAY, r0: 1400, r1: 1600 };
    const width = 720;
    expect(historyX(T0, d, width)).toBe(PAD.left);
    expect(historyX(T0 + 10 * DAY, d, width)).toBe(width - PAD.right);
    expect(historyX(T0 + 5 * DAY, d, width)).toBe((PAD.left + width - PAD.right) / 2);
    // Higher ratings draw higher up.
    expect(historyY(1400, d)).toBe(HEIGHT - PAD.bottom);
    expect(historyY(1600, d)).toBe(PAD.top);
    expect(historyY(1500, d)).toBe((PAD.top + HEIGHT - PAD.bottom) / 2);
  });

  it('draws a single fit (every point at one time) at the left edge, not NaN', () => {
    const series = groupHistory([point('a', 0, 1500), point('b', 0, 1600)]).series;
    const d = historyDomain(series)!;
    expect(d.t0).toBe(d.t1);
    expect(historyX(d.t0, d, 720)).toBe(PAD.left);
    for (const s of series) expect(seriesPath(s, d, 720)).not.toContain('NaN');
  });

  it('builds a path through every point in time order', () => {
    const series = groupHistory([point('a', 0, 1400), point('a', 1, 1600)]).series[0];
    const d = { t0: T0, t1: T0 + DAY, r0: 1400, r1: 1600 };
    expect(seriesPath(series, d, 720)).toBe(
      `M${PAD.left},${HEIGHT - PAD.bottom} L${720 - PAD.right},${PAD.top}`
    );
  });
});
