import { describe, expect, it } from 'vitest';
import { gamesPlayed, matchRows } from './matchScore';

const played = { p1_score_mean: 432.14, p2_score_mean: 418.76, spread_mean: 13.38 };
const row = (rows: ReturnType<typeof matchRows>, label: string) => rows.find((r) => r.label === label)!;

describe('F-MATCH-1 matchRows', () => {
  it("gives each player its own column: player 2's wins are player 1's losses", () => {
    const rows = matchRows({ wins: 12, losses: 8, draws: 1, ...played });
    expect(rows.map((r) => r.label)).toEqual([
      'Wins', 'Draws', 'Score', 'Score %', 'Average score', 'Average spread'
    ]);
    expect(row(rows, 'Wins').values).toEqual(['12', '8']);
    expect(row(rows, 'Draws').values).toEqual(['1', '1']);
    expect(gamesPlayed({ wins: 12, losses: 8, draws: 1, ...played })).toBe(21);
  });

  it('scores a win a point and a draw half of one, and its share of the games', () => {
    const rows = matchRows({ wins: 12, losses: 8, draws: 1, ...played });
    expect(row(rows, 'Score').values).toEqual(['12.5', '8.5']);
    expect(row(rows, 'Score %').values).toEqual(['59.5%', '40.5%']);
    // A whole score has no ".0".
    expect(row(matchRows({ wins: 3, losses: 1, draws: 2, ...played }), 'Score').values).toEqual(['4', '2']);
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
    expect(row(rows, 'Score').values).toEqual(['0', '0']);
    for (const label of ['Score %', 'Average score', 'Average spread']) {
      expect(row(rows, label).values).toEqual(['—', '—']);
      expect(row(rows, label).numbers).toEqual([null, null]);
    }
  });
});
