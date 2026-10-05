import { describe, expect, it } from 'vitest';
import { analysesPerRack, consensusProblem, rackConsensus } from './consensus';

describe('F-CONS-1 analysesPerRack', () => {
  it('says one for a job wanting one analysis per rack, and the range and share otherwise', () => {
    expect(analysesPerRack({ consensus_pct: 100, min_results_per_rack: 1, max_results_per_rack: 1 })).toBe('1');
    expect(analysesPerRack({ consensus_pct: 80, min_results_per_rack: 3, max_results_per_rack: 7 })).toBe(
      '3 to 7, until 80% agree on the best move'
    );
    expect(analysesPerRack({ consensus_pct: 100, min_results_per_rack: 2, max_results_per_rack: 2 })).toBe(
      '2, until 100% agree on the best move'
    );
  });
});

describe('F-CONS-2 rackConsensus', () => {
  const ranked = (analysis: number, ...plays: string[]) =>
    plays.map((move, i) => ({ analysis, rank: i + 1, move }));

  it("counts each analysis's best move, and names the most common", () => {
    const moves = [
      ...ranked(1, '8G WUZ', '8H ZA'),
      ...ranked(2, '8G WUZ', '8D QI'),
      ...ranked(3, '8H ZA', '8G WUZ'),
      ...ranked(4, '8G WUZ')
    ];
    expect(rackConsensus(moves)).toEqual({ analyses: 4, top: '8G WUZ', count: 3, share: '75.0' });
  });

  it('names the alphabetically first move of a tie, and nothing for no analyses', () => {
    expect(rackConsensus([...ranked(1, '8H ZA'), ...ranked(2, '8G WUZ')])?.top).toBe('8G WUZ');
    expect(rackConsensus([])).toBeNull();
  });
});

describe('F-CONS-3 consensusProblem', () => {
  const settings = (min: number, max: number, pct: number) => ({
    min_results_per_rack: min,
    max_results_per_rack: max,
    consensus_pct: pct
  });
  it('accepts what the server accepts', () => {
    expect(consensusProblem(settings(1, 1, 100))).toBeNull();
    // One analysis per rack seeks no agreement, so its share is not checked.
    expect(consensusProblem(settings(1, 1, 10))).toBeNull();
    expect(consensusProblem(settings(2, 5, 80))).toBeNull();
    expect(consensusProblem(settings(3, 3, 100))).toBeNull();
  });
  it('refuses what the server refuses', () => {
    expect(consensusProblem(settings(0, 1, 80))).toMatch(/fewest/);
    expect(consensusProblem(settings(1.5, 2, 80))).toMatch(/fewest/);
    expect(consensusProblem(settings(3, 2, 80))).toMatch(/most/);
    expect(consensusProblem(settings(1, 101, 80))).toMatch(/most/);
    expect(consensusProblem(settings(2, 3, 50))).toMatch(/share/);
    expect(consensusProblem(settings(2, 3, 100.5))).toMatch(/share/);
  });
});
