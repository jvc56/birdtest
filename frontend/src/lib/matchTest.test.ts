import { describe, expect, it } from 'vitest';
import { confidenceProblem, testPct, testScale, testVerdict } from './matchTest';

describe('F-TEST-1 testScale', () => {
  it('puts an even score in the middle, wide enough for the interval, snapped to a round width', () => {
    // 0.51 to 0.55 reaches 5 points from even; with room that is 0.056, so a
    // tenth a side: 40% to 60%.
    const scale = testScale({ lower: 0.51, upper: 0.55, mean: 0.53 });
    expect(scale.ticks.map((t) => t.label)).toEqual(['40%', '45%', '50%', '55%', '60%']);
    expect(scale.ticks.map((t) => t.at)).toEqual([0, 25, 50, 75, 100]);
    expect(scale.lower).toBeCloseTo(55);
    expect(scale.mean).toBeCloseTo(65);
    expect(scale.upper).toBeCloseTo(75);
  });
  it('keeps 50% in the middle when the interval is all on one side, or straddles it', () => {
    expect(testScale({ lower: 0.4, upper: 0.45, mean: 0.425 }).ticks[2]).toEqual({ at: 50, label: '50%' });
    const scale = testScale({ lower: 0.47, upper: 0.53, mean: 0.5 });
    expect(scale.ticks.map((t) => t.label)).toEqual(['45%', '47.5%', '50%', '52.5%', '55%']);
    expect(scale.mean).toBeCloseTo(50);
  });
  it('stops at the scores that exist', () => {
    const scale = testScale({ lower: 0, upper: 1, mean: 0.5 });
    expect([scale.lower, scale.upper, scale.mean]).toEqual([0, 100, 50]);
    expect(scale.ticks.map((t) => t.label)).toEqual(['0%', '25%', '50%', '75%', '100%']);
  });
  it('prints scores to two places', () => {
    expect(testPct(0.57416)).toBe('57.42%');
    expect(testPct(0.5)).toBe('50.00%');
  });
});

describe('F-TEST-2 testVerdict', () => {
  const games = { unit: 'pair' as const, units_completed: 1240, min_units: 500, test: { confidence_pct: 95 } };
  it('says too early before the minimum, and no verdict yet after it', () => {
    expect(testVerdict('running', { ...games, units_completed: 300 })).toMatchObject({
      word: 'Too early', line: 'acts from 500 pairs', chip: 'running', running: true
    });
    expect(testVerdict('running', games)).toMatchObject({ word: 'No verdict yet', running: true });
  });
  it('names the better player, and when it was decided', () => {
    const decided = { status: 'player1_better' as const, lower: 0.534, upper: 0.614, units: 500 };
    expect(testVerdict('player1_better', { ...games, decided })).toMatchObject({
      word: null, player: 0, line: 'is better, decided at 500 pairs', chip: null
    });
    // Found between finish checks: not decided yet, but the test has it.
    expect(testVerdict('player2_better', games)).toMatchObject({ player: 1, line: 'is better at 95% confidence' });
  });
  it('says why there is none: the cap, a pause, or a job completed first', () => {
    expect(testVerdict('inconclusive', games)).toMatchObject({ word: 'Neither better', chip: 'inconclusive' });
    expect(testVerdict('paused', games)).toMatchObject({
      word: 'Paused', line: 'no pairs are being played while the job is inactive', running: false
    });
    expect(testVerdict('undecided', games)).toMatchObject({ word: 'Not decided', chip: 'not decided' });
  });
});

describe('F-TEST-3 confidenceProblem', () => {
  it('takes what the server takes, strictly between 50 and 100', () => {
    for (const pct of [50.05, 50.1, 95, 99.99, 99.995]) expect(confidenceProblem(pct)).toBeNull();
    for (const pct of [50, 100, 49, 120, Number.NaN, Infinity]) {
      expect(confidenceProblem(pct)).toBe('The confidence must be above 50% and below 100%.');
    }
    // A cleared number input binds null.
    expect(confidenceProblem(null)).not.toBeNull();
  });
});
