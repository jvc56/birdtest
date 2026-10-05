import { describe, expect, it } from 'vitest';
import { testBar, testSentence } from './matchTest';
import type { TestResult } from './api';

const result = (over: Partial<TestResult>): TestResult => ({
  mean: 0.53, lower: 0.51, upper: 0.55, confidence_pct: 95, status: 'running', ...over
});

describe('F-TEST-1 testBar', () => {
  it('scales the bar to the interval and an even score, with a margin', () => {
    const bar = testBar({ lower: 0.51, upper: 0.55, mean: 0.53 });
    // The span is 0.50 to 0.55; a tenth of it is under a point, so a point a side.
    expect(bar.from).toBeCloseTo(0.49);
    expect(bar.to).toBeCloseTo(0.56);
    expect(bar.even).toBeCloseTo((0.01 / 0.07) * 100);
    expect(bar.lower).toBeLessThan(bar.mean);
    expect(bar.mean).toBeLessThan(bar.upper);
    expect(bar.even).toBeLessThan(bar.lower);
  });
  it('keeps a wide interval within the scores that exist', () => {
    const bar = testBar({ lower: 0, upper: 1, mean: 0.5 });
    expect([bar.from, bar.to, bar.lower, bar.upper, bar.even]).toEqual([0, 1, 0, 100, 50]);
  });
});

describe('F-TEST-2 testSentence', () => {
  const names: [string, string] = ['simmer', 'static'];
  it('states the score and its range, and nothing in Elo', () => {
    expect(testSentence(result({}), names)).toBe('simmer scores 53.0% per game (95% interval 51.0% to 55.0%).');
  });
  it('names the better player once it is decided, and says when neither is', () => {
    expect(testSentence(result({ status: 'player1_better' }), names)).toMatch(/ simmer is better at 95% confidence\.$/);
    expect(testSentence(result({ status: 'player2_better', mean: 0.47 }), names)).toMatch(
      / static is better at 95% confidence\.$/
    );
    expect(testSentence(result({ status: 'inconclusive' }), names)).toMatch(/Neither is better at 95% confidence/);
  });
});
