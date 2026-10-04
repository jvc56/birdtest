import { describe, expect, it } from 'vitest';
import { gamesPlayed, matchRows } from './matchScore';

const played = { p1_score_mean: 432.14, p2_score_mean: 418.76, spread_mean: 13.38 };
const row = (rows: ReturnType<typeof matchRows>, label: string) => rows.find((r) => r.label === label)!;

describe('F-MATCH-1 matchRows', () => {
  it("has wins, losses, draws, average score and average spread, a column per player", () => {
    const rows = matchRows({ wins: 12, losses: 8, draws: 1, ...played });
    expect(rows.map((r) => r.label)).toEqual(['Wins', 'Losses', 'Draws', 'Average score', 'Average spread']);
    expect(gamesPlayed({ wins: 12, losses: 8, draws: 1, ...played })).toBe(21);
  });

  it("gives player 2 player 1's losses as wins and its wins as losses, fewer losses better", () => {
    const rows = matchRows({ wins: 12, losses: 8, draws: 1, ...played });
    expect(row(rows, 'Wins').values).toEqual(['12', '8']);
    expect(row(rows, 'Losses')).toMatchObject({ values: ['8', '12'], numbers: [8, 12], better: 'lower' });
    expect(row(rows, 'Draws').values).toEqual(['1', '1']);
  });

  it("gives the average scores to a decimal place, and the spread signed from each player's side", () => {
    const rows = matchRows({ wins: 12, losses: 8, draws: 1, ...played });
    expect(row(rows, 'Average score').values).toEqual(['432.1', '418.8']);
    expect(row(rows, 'Average spread').values).toEqual(['+13.4', '-13.4']);
    expect(row(rows, 'Average spread').numbers).toEqual([13.4, -13.4]);
    // Rounds to nothing: neither "+0.0" nor "-0.0", and neither player ahead.
    const even = row(matchRows({ wins: 1, losses: 1, draws: 0, ...played, spread_mean: -0.04 }), 'Average spread');
    expect(even.values).toEqual(['0.0', '0.0']);
    expect(even.numbers).toEqual([0, 0]);
  });

  it('says nothing, rather than 0 or NaN, before any game', () => {
    const rows = matchRows({
      wins: 0, losses: 0, draws: 0, p1_score_mean: null, p2_score_mean: null, spread_mean: null
    });
    expect(row(rows, 'Wins').values).toEqual(['0', '0']);
    for (const label of ['Average score', 'Average spread']) {
      expect(row(rows, label).values).toEqual(['—', '—']);
      expect(row(rows, label).numbers).toEqual([null, null]);
    }
  });
});
