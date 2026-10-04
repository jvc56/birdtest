import { describe, expect, it } from 'vitest';
import { standings } from './compare';

describe('F-CMP-1 standings', () => {
  it('marks the higher value higher and the other lower, in either column', () => {
    expect(standings([3, 1])).toEqual(['higher', 'lower']);
    expect(standings([-2.5, 2.5])).toEqual(['lower', 'higher']);
  });

  it('marks neither when they are equal or one is missing', () => {
    expect(standings([4, 4])).toEqual([null, null]);
    expect(standings([null, 4])).toEqual([null, null]);
    expect(standings([0, null])).toEqual([null, null]);
  });
});
