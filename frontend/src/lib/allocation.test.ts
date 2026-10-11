import { describe, expect, it } from 'vitest';
import { equalShares } from './allocation';

describe('F-ALLOC-1 equalShares', () => {
  it('splits 100% among the jobs above 0%, the remainder to the first', () => {
    const values = { a: 10, b: 0, c: 40, d: 5 };
    expect(equalShares(['a', 'b', 'c', 'd'], values, new Set())).toEqual({ a: 34, b: 0, c: 33, d: 33 });
  });

  it('counts the jobs just made with the running ones', () => {
    // A job at 100% and a three-job round robin at 0%: before, nothing changed.
    const values = { old: 100, x: 0, y: 0, z: 0 };
    expect(equalShares(['x', 'y', 'z', 'old'], values, new Set(['x', 'y', 'z']))).toEqual({
      x: 25,
      y: 25,
      z: 25,
      old: 25
    });
    // With the old job set to 0% first, the round robin shares it all.
    expect(equalShares(['x', 'y', 'z', 'old'], { ...values, old: 0 }, new Set(['x', 'y', 'z']))).toEqual({
      x: 34,
      y: 33,
      z: 33,
      old: 0
    });
  });

  it('shares among every job when none is chosen, and leaves no jobs alone', () => {
    expect(equalShares(['a', 'b', 'c'], { a: 0, b: 0, c: 0 }, new Set())).toEqual({ a: 34, b: 33, c: 33 });
    expect(equalShares([], {}, new Set(['gone']))).toEqual({});
  });
});
