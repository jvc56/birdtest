import { describe, expect, it } from 'vitest';
import { pairOutcomes, pairShare } from './pentanomial';

const names: [string, string] = ['simmer', 'static'];

describe('F-CHART-7 pair outcomes, player 1 to player 2', () => {
  it("reads each part from the side it favours: player 1's lost-both is player 2's won-both", () => {
    // Player 1 won every pair: bucket 4, the first part.
    const sweep = pairOutcomes([0, 0, 0, 0, 12], 12, names);
    expect(sweep.map((o) => o.label)).toEqual([
      'simmer won both', 'simmer won one, drew one', 'Even', 'static won one, drew one', 'static won both'
    ]);
    expect(sweep.map((o) => o.count)).toEqual([12, 0, 0, 0, 0]);
    // Player 2 won every pair: bucket 0, the last part.
    expect(pairOutcomes([12, 0, 0, 0, 0], 12, names).map((o) => o.count)).toEqual([0, 0, 0, 0, 12]);
  });

  it('holds all five buckets once, each for the player it favours', () => {
    const outcomes = pairOutcomes([1, 2, 3, 4, 5], 15, names);
    expect(outcomes.map((o) => o.bucket)).toEqual([4, 3, 2, 1, 0]);
    expect(outcomes.map((o) => o.player)).toEqual([0, 0, null, 1, 1]);
    expect(outcomes.map((o) => o.both)).toEqual([true, false, false, false, true]);
  });
});

describe('F-CHART-8 pair-outcome percentages', () => {
  it('are computed against pairs, not games', () => {
    // 100 pairs are 200 games; a games denominator would halve every share.
    expect(pairOutcomes([10, 20, 40, 20, 10], 100, names).map((o) => o.share)).toEqual([
      '10.0', '20.0', '40.0', '20.0', '10.0'
    ]);
  });

  it('round to one decimal place', () => {
    expect(pairShare(1, 3)).toBe('33.3');
    expect(pairShare(2, 3)).toBe('66.7');
    expect(pairShare(1, 8)).toBe('12.5');
  });

  it('render 0.0% rather than NaN for a zero denominator', () => {
    for (const o of pairOutcomes([0, 0, 0, 0, 0], 0, names)) expect(o.share).toBe('0.0');
    expect(pairShare(0, 0)).toBe('0.0');
    // Nor Infinity, if a count ever arrives ahead of the pair total.
    expect(pairShare(3, 0)).toBe('0.0');
  });
});
