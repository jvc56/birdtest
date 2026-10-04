import { describe, expect, it } from 'vitest';
import { pairShare, PAIR_OUTCOMES, pentanomialRows } from './pentanomial';

describe('F-CHART-7 pair outcomes, a column per player', () => {
  it("reads each player's row from its own side: player 1's lost-both is player 2's won-both", () => {
    // Player 1 won every pair: bucket 4.
    const sweep = pentanomialRows([0, 0, 0, 0, 12], 12);
    expect(sweep.map((r) => r.label)).toEqual(['Won both', 'Won one, drew one', 'Even']);
    expect(sweep[0].numbers).toEqual([12, 0]);
    expect(sweep[0].values).toEqual(['12 (100.0%)', '0 (0.0%)']);
    // Player 2 won every pair: bucket 0.
    expect(pentanomialRows([12, 0, 0, 0, 0], 12)[0].numbers).toEqual([0, 12]);
  });

  it('holds all five buckets, each once per player, and the even bucket for both', () => {
    const rows = pentanomialRows([1, 2, 3, 4, 5], 15);
    expect(rows.map((r) => r.numbers)).toEqual([
      [5, 1],
      [4, 2],
      [3, 3]
    ]);
    const seen = PAIR_OUTCOMES.flatMap((o) => o.buckets);
    expect(new Set(seen)).toEqual(new Set([0, 1, 2, 3, 4]));
  });
});

describe('F-CHART-8 pair-outcome percentages', () => {
  it('are computed against pairs, not games', () => {
    // 100 pairs are 200 games; a games denominator would halve every share.
    const rows = pentanomialRows([10, 20, 40, 20, 10], 100);
    expect(rows.map((r) => r.values)).toEqual([
      ['10 (10.0%)', '10 (10.0%)'],
      ['20 (20.0%)', '20 (20.0%)'],
      ['40 (40.0%)', '40 (40.0%)']
    ]);
  });

  it('round to one decimal place', () => {
    expect(pairShare(1, 3)).toBe('33.3');
    expect(pairShare(2, 3)).toBe('66.7');
    expect(pairShare(1, 8)).toBe('12.5');
  });

  it('render 0.0% rather than NaN for a zero denominator', () => {
    const rows = pentanomialRows([0, 0, 0, 0, 0], 0);
    for (const r of rows) expect(r.values).toEqual(['0 (0.0%)', '0 (0.0%)']);
    expect(pairShare(0, 0)).toBe('0.0');
    // Nor Infinity, if a count ever arrives ahead of the pair total.
    expect(pairShare(3, 0)).toBe('0.0');
  });
});
