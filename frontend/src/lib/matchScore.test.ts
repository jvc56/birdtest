import { describe, expect, it } from 'vitest';
import { gamesPlayed, matchScore } from './matchScore';

const played = { p1_score_mean: 432.144, p2_score_mean: 418.756, spread_mean: 13.388 };

describe('F-MATCH-1 matchScore', () => {
  it("gives each player's wins, player 2's being player 1's losses, and the draws once", () => {
    const score = matchScore({ wins: 12, losses: 8, draws: 1, ...played });
    expect(score).toMatchObject({ played: 21, wins: [12, 8], draws: 1, leader: 1 });
    expect(gamesPlayed({ wins: 12, losses: 8, draws: 1, ...played })).toBe(21);
    expect(matchScore({ wins: 8, losses: 12, draws: 1, ...played }).leader).toBe(2);
    expect(matchScore({ wins: 8, losses: 8, draws: 1, ...played }).leader).toBe(0);
  });

  it("gives each player's share of the points to two places, a draw counting half", () => {
    // 12.5 of 21 is 59.523...%, and player 2 the rest.
    expect(matchScore({ wins: 12, losses: 8, draws: 1, ...played }).shares).toEqual(['59.52%', '40.48%']);
  });

  it('gives the average scores to two places, and the spread as what the player ahead leads by', () => {
    const score = matchScore({ wins: 12, losses: 8, draws: 1, ...played });
    expect(score.averages).toEqual(['432.14', '418.76']);
    expect(score.spread).toEqual({ points: '13.39', ahead: 1 });
    expect(matchScore({ wins: 8, losses: 12, draws: 1, ...played, spread_mean: -2.5 }).spread).toEqual({
      points: '2.50',
      ahead: 2
    });
    // Rounds to nothing: "0.00", never "-0.00", and neither player ahead.
    expect(matchScore({ wins: 1, losses: 1, draws: 0, ...played, spread_mean: -0.004 }).spread).toEqual({
      points: '0.00',
      ahead: 0
    });
  });

  it('says nothing, rather than 0 or NaN, before any game', () => {
    const score = matchScore({
      wins: 0, losses: 0, draws: 0, p1_score_mean: null, p2_score_mean: null, spread_mean: null
    });
    expect(score).toMatchObject({ played: 0, wins: [0, 0], shares: null, leader: 0, spread: null });
    expect(score.averages).toEqual(['—', '—']);
  });
});
