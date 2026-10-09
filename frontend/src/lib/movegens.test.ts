import { describe, expect, it } from 'vitest';
import { MOVEGEN_TYPES, contributorKey, movegensLines } from './movegens';

describe('F-MOVEGENS-1 movegensLines', () => {
  it('lists every job type in the same order, labelled', () => {
    const lines = movegensLines({ opening_rack: 1, games: 2, game_pairs: 3, leave_generation: 4 });
    expect(lines.map((line) => line.type)).toEqual(MOVEGEN_TYPES);
    expect(lines.map((line) => line.label)).toEqual([
      'Opening Rack Analysis',
      'Games',
      'Game Pairs',
      'Leave Generation'
    ]);
    expect(lines.map((line) => line.movegens)).toEqual([1, 2, 3, 4]);
  });

  it('reads a type the answer leaves out as 0, so all four are always listed', () => {
    expect(movegensLines({ games: 7 }).map((line) => line.movegens)).toEqual([0, 7, 0, 0]);
    expect(movegensLines({})).toHaveLength(4);
  });
});

describe('F-MOVEGENS-2 contributorKey', () => {
  it('names an account by its id and an anonymous worker by its pseudonym', () => {
    expect(contributorKey({ user_id: '3f2b8c1e-9d4a-4e7b-a1c2-5d6e7f809a1b', anon_id: null })).toBe(
      'user/3f2b8c1e-9d4a-4e7b-a1c2-5d6e7f809a1b'
    );
    expect(contributorKey({ user_id: null, anon_id: '0123456789abcdef' })).toBe('anon/0123456789abcdef');
  });

  it('escapes what it puts in the path, and has no key for a row naming nobody', () => {
    expect(contributorKey({ user_id: null, anon_id: 'a/b?c' })).toBe('anon/a%2Fb%3Fc');
    expect(contributorKey({ user_id: null, anon_id: null })).toBeNull();
  });
});
