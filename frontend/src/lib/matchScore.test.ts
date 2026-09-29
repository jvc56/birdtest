import { describe, expect, it } from 'vitest';
import { matchScore } from './matchScore';

const played = { p1_score_mean: 432.14, p2_score_mean: 418.76, spread_mean: 13.38 };

describe('F-MATCH-1 matchScore', () => {
  it("scores player 1's record: a win a point, a draw half of one", () => {
    const m = matchScore({ wins: 12, losses: 8, draws: 1, ...played });
    expect(m.record).toBe('12–8–1');
    expect(m.games).toBe(21);
    expect(m.score).toBe('12.5');
    expect(m.scorePct).toBe('59.5%');
    // A whole score has no ".0".
    expect(matchScore({ wins: 3, losses: 1, draws: 2, ...played }).score).toBe('4');
  });

  it("gives the players' average scores and the spread to a decimal place, signed", () => {
    const m = matchScore({ wins: 12, losses: 8, draws: 1, ...played });
    expect([m.p1Mean, m.p2Mean, m.spread]).toEqual(['432.1', '418.8', '+13.4']);
    expect(matchScore({ wins: 1, losses: 3, draws: 0, ...played, spread_mean: -2.5 }).spread).toBe('-2.5');
    // Rounds to nothing: neither "+0.0" nor "-0.0".
    expect(matchScore({ wins: 1, losses: 1, draws: 0, ...played, spread_mean: -0.04 }).spread).toBe('0.0');
  });

  it('says nothing, rather than 0 or NaN, before any game', () => {
    const m = matchScore({
      wins: 0, losses: 0, draws: 0, p1_score_mean: null, p2_score_mean: null, spread_mean: null
    });
    expect(m.record).toBe('0–0–0');
    expect(m.score).toBe('0');
    expect([m.scorePct, m.p1Mean, m.p2Mean, m.spread]).toEqual([null, null, null, null]);
  });
});
