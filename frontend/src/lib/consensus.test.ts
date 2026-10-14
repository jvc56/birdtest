import { describe, expect, it } from 'vitest';
import {
  analysesPerRack,
  consensusFields,
  consensusProblem,
  consensusStage,
  rackConsensus,
  type ConsensusSettings,
  type RackStage
} from './consensus';

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
  it('accepts what the server accepts of the fields sent', () => {
    expect(consensusProblem(settings(1, 1, 100))).toBeNull();
    expect(consensusProblem(settings(2, 5, 80))).toBeNull();
    expect(consensusProblem(settings(3, 3, 100))).toBeNull();
    expect(consensusProblem(settings(2, 3, 50.5))).toBeNull();
    // At one analysis per rack the share is not sent, so a share the server
    // would refuse is not checked: it checks its own default or stored one.
    expect(consensusProblem(settings(1, 1, 10))).toBeNull();
  });
  it('refuses what the server refuses of the fields sent', () => {
    expect(consensusProblem(settings(0, 1, 80))).toMatch(/fewest/);
    expect(consensusProblem(settings(1.5, 2, 80))).toMatch(/fewest/);
    expect(consensusProblem(settings(3, 2, 80))).toMatch(/most/);
    expect(consensusProblem(settings(1, 101, 80))).toMatch(/most/);
    expect(consensusProblem(settings(2, 3, 50))).toMatch(/share/);
    expect(consensusProblem(settings(2, 3, 100.5))).toMatch(/share/);
    expect(consensusProblem(settings(2, 3, NaN))).toMatch(/share/);
  });
  it('checks the fewest and most whatever the most', () => {
    // The job form once sent these as one analysis per rack, unrefused.
    expect(consensusProblem(settings(3, 1, 80))).toMatch(/most/);
    expect(consensusProblem(settings(1, 0, 80))).toMatch(/most/);
  });
});

describe('F-CONS-4 consensusFields', () => {
  it('sends the share only past one analysis per rack', () => {
    expect(
      consensusFields({ min_results_per_rack: 1, max_results_per_rack: 1, consensus_pct: 40 })
    ).toEqual({ min_results_per_rack: 1, max_results_per_rack: 1 });
    expect(
      consensusFields({ min_results_per_rack: 2, max_results_per_rack: 5, consensus_pct: 80 })
    ).toEqual({ min_results_per_rack: 2, max_results_per_rack: 5, consensus_pct: 80 });
  });
  it('sends numbers whatever the input bound', () => {
    const typed = { min_results_per_rack: '2', max_results_per_rack: '3', consensus_pct: '75' };
    expect(consensusFields(typed as unknown as ConsensusSettings)).toEqual({
      min_results_per_rack: 2,
      max_results_per_rack: 3,
      consensus_pct: 75
    });
  });
});

describe('F-CONS-5 consensusStage', () => {
  const stage = (over: Partial<RackStage> = {}) => ({
    racks_analyzed: 2060000,
    racks_settled: 0,
    racks_without_consensus: 0,
    racks_total: 3199724,
    fewest_open_analyses: null,
    min_results_per_rack: 3,
    max_results_per_rack: 7,
    ...over
  });

  it('says nothing for a job analysing each rack once', () => {
    expect(consensusStage(stage({ min_results_per_rack: 1, max_results_per_rack: 1 }), false)).toBeNull();
  });

  it('reads the first pass, and when no rack can settle yet', () => {
    expect(consensusStage(stage(), false)).toEqual({
      stage: 'First pass.',
      text: '64.4% of racks have their first analysis. None can settle before its 3rd. Re-analysis starts when every rack has one.'
    });
    // A rack can settle at its first analysis when the fewest is one.
    expect(consensusStage(stage({ min_results_per_rack: 1 }), false)?.text).toBe(
      '64.4% of racks have their first analysis. Re-analysis starts when every rack has one.'
    );
  });

  it('reads the re-analysis from the fewest analyses an open rack has', () => {
    const s = stage({ racks_analyzed: 3199724, racks_settled: 1144000, fewest_open_analyses: 2 });
    expect(consensusStage(s, false)).toEqual({
      stage: 'Re-analysis.',
      text: 'Every open rack has at least 2 analyses, and racks with the fewest are analysed first.'
    });
  });

  it('reads finished, agreed and not', () => {
    const s = stage({ racks_analyzed: 3199724, racks_settled: 3199724, racks_without_consensus: 24967 });
    expect(consensusStage(s, true)).toEqual({
      stage: 'Finished.',
      text: `Every rack is settled: ${(3174757).toLocaleString()} agreed, ${(24967).toLocaleString()} without a consensus after 7 analyses.`
    });
  });
});
