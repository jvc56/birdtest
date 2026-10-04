import { describe, expect, it } from 'vitest';
import { standings } from './compare';

describe('F-CMP-1 standings', () => {
  it('marks the higher value better and the other worse, in either column', () => {
    expect(standings([3, 1])).toEqual(['better', 'worse']);
    expect(standings([-2.5, 2.5])).toEqual(['worse', 'better']);
  });

  it('marks the lower value better where fewer is better, as losses are', () => {
    expect(standings([3, 1], 'lower')).toEqual(['worse', 'better']);
    expect(standings([0, 2], 'lower')).toEqual(['better', 'worse']);
  });

  it('marks neither when they are equal or one is missing', () => {
    expect(standings([4, 4])).toEqual([null, null]);
    expect(standings([null, 4])).toEqual([null, null]);
    expect(standings([0, null], 'lower')).toEqual([null, null]);
  });
});
