import { describe, expect, it } from 'vitest';
import { pairShare, PENTANOMIAL_LABELS, pentanomialRows } from './pentanomial';

describe('F-CHART-7 pentanomial bucket labels', () => {
  it('index 0 is "P1 lost both" and index 4 "won both"', () => {
    expect(PENTANOMIAL_LABELS).toHaveLength(5);
    expect(PENTANOMIAL_LABELS[0]).toBe('P1 lost both');
    expect(PENTANOMIAL_LABELS[4]).toBe('P1 won both');
    expect(PENTANOMIAL_LABELS[4]).toMatch(/won both$/);
  });

  it('labels every bucket by player 1\'s half-points across the pair', () => {
    // 0 = L+L, 1 = L+D, 2 = W+L or D+D, 3 = W+D, 4 = W+W.
    expect(PENTANOMIAL_LABELS).toEqual([
      'P1 lost both',
      'Lost one, drew one',
      'Split 1-1',
      'Won one, drew one',
      'P1 won both'
    ]);
  });

  it('puts each count beside its own label, in bucket order', () => {
    // Player 1 won every pair: all of it belongs in bucket 4, "won both".
    const sweep = pentanomialRows([0, 0, 0, 0, 12], 12);
    expect(sweep.map((r) => r.label)).toEqual([...PENTANOMIAL_LABELS]);
    expect(sweep[4]).toEqual({ label: 'P1 won both', pairs: 12, share: '100.0' });
    expect(sweep[0]).toEqual({ label: 'P1 lost both', pairs: 0, share: '0.0' });

    const rows = pentanomialRows([1, 2, 3, 4, 5], 15);
    expect(rows.map((r) => [r.label, r.pairs])).toEqual([
      ['P1 lost both', 1],
      ['Lost one, drew one', 2],
      ['Split 1-1', 3],
      ['Won one, drew one', 4],
      ['P1 won both', 5]
    ]);
  });
});

describe('F-CHART-8 pentanomial percentages', () => {
  it('are computed against pairs, not games', () => {
    // 100 pairs are 200 games; a games denominator would halve every share.
    const rows = pentanomialRows([10, 20, 40, 20, 10], 100);
    expect(rows.map((r) => r.share)).toEqual(['10.0', '20.0', '40.0', '20.0', '10.0']);
    const total = rows.reduce((sum, r) => sum + Number(r.share), 0);
    expect(total).toBeCloseTo(100, 9);
  });

  it('round to one decimal place', () => {
    expect(pairShare(1, 3)).toBe('33.3');
    expect(pairShare(2, 3)).toBe('66.7');
    expect(pairShare(1, 8)).toBe('12.5');
  });

  it('render 0.0% rather than NaN for a zero denominator', () => {
    const rows = pentanomialRows([0, 0, 0, 0, 0], 0);
    expect(rows.map((r) => r.share)).toEqual(['0.0', '0.0', '0.0', '0.0', '0.0']);
    for (const r of rows) expect(r.share).not.toContain('NaN');
    expect(pairShare(0, 0)).toBe('0.0');
    // Nor Infinity, if a count ever arrives ahead of the pair total.
    expect(pairShare(3, 0)).toBe('0.0');
  });
});
