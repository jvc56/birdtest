import { describe, expect, it } from 'vitest';
import { drawShare, inferenceSummary, plyAt, plyColumns, plyHeaders } from './moveList';

const ply = (p: number) => ({ ply: p, bingo_percentage: 10 + p, average_score: 30 + p });

describe('F-MOVES-1 ply columns', () => {
  it('shows up to two plies: the most any move has', () => {
    expect(plyColumns([{ plies: [] }, { plies: [] }])).toBe(0);
    expect(plyColumns([{ plies: [ply(0)] }, { plies: [] }])).toBe(1);
    expect(plyColumns([{ plies: [ply(0), ply(1), ply(2), ply(3)] }])).toBe(2);
    expect(plyColumns([])).toBe(0);
  });
  it('names them P1-S, P1-BP, P2-S, P2-BP, P1 the reply', () => {
    expect(plyHeaders(2).map((h) => h.label)).toEqual(['P1-S', 'P1-BP', 'P2-S', 'P2-BP']);
    expect(plyHeaders(1)[0].title).toBe("Ply 1: the reply's average score");
    expect(plyHeaders(0)).toEqual([]);
  });
  it('finds a ply by its number, not its place', () => {
    expect(plyAt([ply(1)], 0)).toBeNull();
    expect(plyAt([ply(1)], 1)).toEqual(ply(1));
  });
});

describe('F-MOVES-2 inference', () => {
  const inference = { num_leaves: 143, total_draws: 900, average_equity: 12.44, leaves: [] };
  it('says how many leaves it found, from what, and their mean equity', () => {
    expect(inferenceSummary(inference, 'O6 BHUT')).toBe(
      'Inferred from O6 BHUT: 143 possible leaves, average equity 12.4'
    );
    expect(inferenceSummary({ ...inference, num_leaves: 1 }, null)).toBe(
      'Inferred: 1 possible leave, average equity 12.4'
    );
  });
  it('shows each leave as a share of the draws', () => {
    expect(drawShare(120, 960)).toBe('12.5%');
    expect(drawShare(1, 0)).toBe('—');
  });
});
