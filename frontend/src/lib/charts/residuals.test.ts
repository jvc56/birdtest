import { describe, expect, it } from 'vitest';
import type { RatingHeadToHead } from '$lib/api';
import { formatResidual, isNonTransitive, NOTABLE, notableResiduals } from './residuals';

function cell(row: string, col: string, actual: number, predicted: number): RatingHeadToHead {
  return { row, col, pairs: 1000, actual, predicted, stderr: 0.01, spread: 0 };
}

describe('F-CHART-6 the non-transitivity warning', () => {
  const big = (row: string, col: string, sign: 1 | -1) => cell(row, col, 0.5 + sign * 0.2, 0.5);
  const small = (row: string, col: string) => cell(row, col, 0.52, 0.5);

  it('is not raised with fewer than three head-to-heads past the threshold', () => {
    expect(isNonTransitive([])).toBe(false);
    expect(isNonTransitive([big('a', 'b', 1)])).toBe(false);
    expect(isNonTransitive([big('a', 'b', 1), big('b', 'c', -1)])).toBe(false);
    // Any number of small ones does not add up to a flag.
    const many = Array.from({ length: 20 }, (_, i) => small(`r${i}`, `c${i}`));
    expect(isNonTransitive([...many, big('a', 'b', 1), big('b', 'c', 1)])).toBe(false);
  });

  it('is raised at three, counting both signs', () => {
    // The rock-paper-scissors triangle: a beats b, b beats c, c beats a, all
    // far from what any scalar rating predicts.
    const triangle = [big('a', 'b', 1), big('b', 'c', 1), big('a', 'c', -1)];
    expect(notableResiduals(triangle)).toHaveLength(3);
    expect(isNonTransitive(triangle)).toBe(true);
    expect(isNonTransitive([...triangle, small('x', 'y')])).toBe(true);
  });

  it('is not raised by residuals too few pairs stand behind', () => {
    // Twenty points off on ten pairs each is well inside sampling noise.
    const young = [big('a', 'b', 1), big('b', 'c', 1), big('a', 'c', -1)].map((c) => ({
      ...c,
      pairs: 10
    }));
    expect(notableResiduals(young)).toHaveLength(3);
    expect(isNonTransitive(young)).toBe(false);
    // The same misses on a thousand pairs each are not.
    expect(isNonTransitive(young.map((c) => ({ ...c, pairs: 1000 })))).toBe(true);
  });

  it('counts a residual as notable just past the threshold and not just below it', () => {
    expect(NOTABLE).toBe(0.05);
    expect(notableResiduals([cell('a', 'b', 0.551, 0.5)])).toHaveLength(1);
    expect(notableResiduals([cell('a', 'b', 0.449, 0.5)])).toHaveLength(1);
    expect(notableResiduals([cell('a', 'b', 0.549, 0.5)])).toHaveLength(0);
    expect(notableResiduals([cell('a', 'b', 0.451, 0.5)])).toHaveLength(0);
  });
});

describe('F-CHART-6 a residual as a hover states it', () => {
  it('labels in signed percentage points', () => {
    expect(formatResidual(0.075)).toBe('+7.5');
    expect(formatResidual(-0.03)).toBe('-3.0');
    expect(formatResidual(0)).toBe('0.0');
  });
});
