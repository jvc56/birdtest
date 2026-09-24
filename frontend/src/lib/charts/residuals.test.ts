import { describe, expect, it } from 'vitest';
import type { RatingResidual } from '$lib/api';
import {
  formatResidual,
  isNonTransitive,
  magnitude,
  NOTABLE,
  notableResiduals,
  residual,
  residualBar,
  residualFill,
  sortResiduals
} from './residuals';

function cell(row: string, col: string, actual: number, predicted: number): RatingResidual {
  return { row, col, pairs: 1000, actual, predicted };
}

describe('F-CHART-6 ResidualMatrix ordering', () => {
  it('sorts by absolute residual, descending, whatever the sign', () => {
    const cells = [
      cell('a', 'b', 0.52, 0.5), // +0.02
      cell('a', 'c', 0.3, 0.5), // -0.20
      cell('b', 'c', 0.61, 0.5), // +0.11
      cell('c', 'd', 0.45, 0.5) // -0.05
    ];
    expect(sortResiduals(cells).map((c) => residual(c))).toEqual([
      0.3 - 0.5,
      0.61 - 0.5,
      0.45 - 0.5,
      0.52 - 0.5
    ]);
  });

  it('breaks ties by row then column, as the API does, and leaves the input alone', () => {
    const cells = [cell('b', 'c', 0.6, 0.5), cell('a', 'd', 0.4, 0.5), cell('a', 'c', 0.6, 0.5)];
    const snapshot = [...cells];
    expect(sortResiduals(cells).map((c) => `${c.row}${c.col}`)).toEqual(['ac', 'ad', 'bc']);
    expect(cells).toEqual(snapshot);
  });

  it('is a no-op on input the API already sorted', () => {
    const cells = [cell('a', 'b', 0.8, 0.5), cell('a', 'c', 0.35, 0.5), cell('b', 'c', 0.5, 0.49)];
    expect(sortResiduals(cells)).toEqual(cells);
  });
});

describe('F-CHART-6 ResidualMatrix non-transitivity flag', () => {
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

  it('counts a residual as notable just past the threshold and not just below it', () => {
    expect(NOTABLE).toBe(0.05);
    expect(notableResiduals([cell('a', 'b', 0.551, 0.5)])).toHaveLength(1);
    expect(notableResiduals([cell('a', 'b', 0.449, 0.5)])).toHaveLength(1);
    expect(notableResiduals([cell('a', 'b', 0.549, 0.5)])).toHaveLength(0);
    expect(notableResiduals([cell('a', 'b', 0.451, 0.5)])).toHaveLength(0);
  });
});

describe('ResidualMatrix bar and label', () => {
  it('grows the bar right for a positive residual and left for a negative one', () => {
    expect(residualBar(0.125)).toEqual({ left: 50, width: 25 });
    expect(residualBar(-0.125)).toEqual({ left: 25, width: 25 });
    expect(residualBar(0)).toEqual({ left: 50, width: 0 });
  });

  it('saturates at the scale maximum rather than overflowing the track', () => {
    expect(magnitude(0.9)).toBe(1);
    expect(residualBar(0.9)).toEqual({ left: 50, width: 50 });
    expect(residualBar(-0.9)).toEqual({ left: 0, width: 50 });
  });

  it('colours warm above prediction, cool below, grey near zero', () => {
    expect(residualFill(0)).toBe('hsl(217 19% 20%)');
    expect(residualFill(0.005)).toBe('hsl(217 19% 20%)');
    expect(residualFill(0.25)).toBe('hsl(25 85% 52%)');
    expect(residualFill(-0.25)).toBe('hsl(205 85% 52%)');
  });

  it('labels in signed percentage points', () => {
    expect(formatResidual(0.075)).toBe('+7.5');
    expect(formatResidual(-0.03)).toBe('-3.0');
    expect(formatResidual(0)).toBe('0.0');
  });
});
