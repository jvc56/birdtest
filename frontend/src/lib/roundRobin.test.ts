import { describe, expect, it } from 'vitest';
import { MAX_ROUND_ROBIN_CONFIGS, matchupSummary, matchups, matchupsAllowed } from './roundRobin';

const name = (player: string) => player.toUpperCase();
const seats = (list: { player1: string; player2: string }[]) =>
  list.map((m) => `${m.player1}-${m.player2}`);

describe('F-RR-1 matchups', () => {
  it('makes one self-play job of one config, under the name as given', () => {
    expect(matchups(['a'], '  solo  ', name)).toEqual([{ player1: 'a', player2: 'a', name: 'solo' }]);
  });

  it('makes every pairing once, seated in the order the configs are listed', () => {
    expect(seats(matchups(['a', 'b'], 'x', name))).toEqual(['a-b']);
    expect(seats(matchups(['c', 'a', 'b'], 'x', name))).toEqual(['c-a', 'c-b', 'a-b']);
    const four = matchups(['a', 'b', 'c', 'd'], 'x', name);
    expect(seats(four)).toEqual(['a-b', 'a-c', 'a-d', 'b-c', 'b-d', 'c-d']);
  });

  it('names each job "{name}: A vs B", or "A vs B" with no name, as the server does', () => {
    expect(matchups(['a', 'b', 'c'], ' trio ', name).map((m) => m.name)).toEqual([
      'trio: A vs B',
      'trio: A vs C',
      'trio: B vs C'
    ]);
    expect(matchups(['a', 'b'], '', name).map((m) => m.name)).toEqual(['A vs B']);
  });

  it('makes nothing of nothing', () => {
    expect(matchups([], 'x', name)).toEqual([]);
  });
});

describe('F-RR-2 matchupSummary', () => {
  it('counts the jobs, and says why none or too many cannot be sent', () => {
    expect(matchupSummary(1)).toBe('1 config → 1 self-play job');
    expect(matchupSummary(2)).toBe('2 configs → 1 job');
    expect(matchupSummary(4)).toBe('4 configs → 6 jobs');
    expect(matchupSummary(12)).toBe('12 configs → 66 jobs');
    expect(matchupSummary(0)).toBe('Tick at least one player config.');
    expect(matchupSummary(13)).toBe('At most 12 player configs: 13 are ticked.');
    expect([0, 1, 12, 13].map(matchupsAllowed)).toEqual([false, true, true, false]);
    expect(MAX_ROUND_ROBIN_CONFIGS).toBe(12);
  });
});
