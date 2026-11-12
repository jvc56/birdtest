import { describe, expect, it } from 'vitest';
import {
  analysesPerRack,
  consensusFields,
  consensusProblem,
  consensusStage,
  fewestToSettle,
  rackConsensus,
  versionAtLeast,
  type ConsensusSettings,
  type RackStage
} from './consensus';

const settings = (max: number, margin = 2, top = 10): ConsensusSettings => ({
  max_results_per_rack: max,
  reissue_top_moves: top,
  win_margin: margin
});

describe('F-CONS-1 analysesPerRack', () => {
  it('says one for a job wanting one analysis per rack, and the most, lead and moves otherwise', () => {
    expect(analysesPerRack(settings(1))).toBe('1');
    expect(analysesPerRack(settings(7, 2, 10))).toBe(
      "up to 7, until one move leads by 2; later analyses rank the first's top 10 moves"
    );
  });
});

describe('F-CONS-2 rackConsensus', () => {
  const ranked = (analysis: number, ...plays: string[]) =>
    plays.map((move, i) => ({ analysis, rank: i + 1, move }));

  it("counts each analysis's best move, and names the most common and the runner-up's count", () => {
    const moves = [
      ...ranked(1, '8G WUZ', '8H ZA'),
      ...ranked(2, '8G WUZ', '8D QI'),
      ...ranked(3, '8H ZA', '8G WUZ'),
      ...ranked(4, '8G WUZ')
    ];
    expect(rackConsensus(moves)).toEqual({ analyses: 4, top: '8G WUZ', count: 3, runnerUp: 1, share: '75.0' });
    expect(rackConsensus(ranked(1, '8G WUZ', '8H ZA'))?.runnerUp).toBe(0);
  });

  it('names the alphabetically first move of a tie, and nothing for no analyses', () => {
    const tie = rackConsensus([...ranked(1, '8H ZA'), ...ranked(2, '8G WUZ')]);
    expect([tie?.top, tie?.count, tie?.runnerUp]).toEqual(['8G WUZ', 1, 1]);
    expect(rackConsensus([])).toBeNull();
  });
});

describe('F-CONS-3 consensusProblem', () => {
  it('accepts what the server accepts', () => {
    expect(consensusProblem(settings(1))).toBeNull();
    expect(consensusProblem(settings(7, 2, 10))).toBeNull();
    expect(consensusProblem(settings(100, 100, 100))).toBeNull();
    expect(consensusProblem(settings(3, 1, 2))).toBeNull();
  });
  it('refuses what the server refuses', () => {
    expect(consensusProblem(settings(0))).toMatch(/most/);
    expect(consensusProblem(settings(101))).toMatch(/most/);
    expect(consensusProblem(settings(1.5))).toMatch(/most/);
    expect(consensusProblem(settings(3, 2, 1))).toMatch(/moves reanalysed/);
    expect(consensusProblem(settings(3, 2, 101))).toMatch(/moves reanalysed/);
    expect(consensusProblem(settings(3, 0))).toMatch(/lead/);
    expect(consensusProblem(settings(3, 101))).toMatch(/lead/);
    expect(consensusProblem(settings(3, NaN))).toMatch(/lead/);
  });
});

describe('F-CONS-4 consensusFields', () => {
  it('sends numbers whatever the input bound', () => {
    const typed = { max_results_per_rack: '3', reissue_top_moves: '12', win_margin: '2' };
    expect(consensusFields(typed as unknown as ConsensusSettings)).toEqual(settings(3, 2, 12));
  });
});

describe('F-CONS-5 consensusStage', () => {
  const stage = (over: Partial<RackStage> = {}): RackStage => ({
    racks_analyzed: 2060000,
    racks_settled: 0,
    racks_without_consensus: 0,
    racks_total: 3199724,
    fewest_open_analyses: null,
    max_results_per_rack: 7,
    win_margin: 3,
    reissue_top_moves: 10,
    ...over
  });

  it('says nothing for a job analysing each rack once', () => {
    expect(consensusStage(stage({ max_results_per_rack: 1 }), false)).toBeNull();
  });

  it('reads the first pass, and when no rack can settle yet', () => {
    expect(consensusStage(stage(), false)).toEqual({
      stage: 'First pass.',
      text: '64.4% of racks have their first analysis. None can settle before its 3rd. Re-analysis starts when every rack has one.'
    });
    // A lead of one is the first analysis alone.
    expect(consensusStage(stage({ win_margin: 1 }), false)?.text).toBe(
      '64.4% of racks have their first analysis. Re-analysis starts when every rack has one.'
    );
  });

  it('reads the re-analysis from the fewest analyses an open rack has', () => {
    const s = stage({ racks_analyzed: 3199724, racks_settled: 1144000, fewest_open_analyses: 2 });
    expect(consensusStage(s, false)).toEqual({
      stage: 'Re-analysis.',
      text: "Every open rack has at least 2 analyses, and racks with the fewest are analysed first, over their first analysis's top 10 moves. A rack settles when one move leads by 3."
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

describe('F-CONS-6 fewestToSettle and versionAtLeast', () => {
  it('needs as many analyses as the lead, at most the most, and one for one analysis a rack', () => {
    expect(fewestToSettle(settings(7, 2))).toBe(2);
    expect(fewestToSettle(settings(2, 5))).toBe(2);
    expect(fewestToSettle(settings(1, 5))).toBe(1);
  });
  it('compares versions as numbers', () => {
    expect(versionAtLeast('0.1.5', '0.1.5')).toBe(true);
    expect(versionAtLeast('0.1.10', '0.1.5')).toBe(true);
    expect(versionAtLeast('0.2', '0.1.5')).toBe(true);
    expect(versionAtLeast('0.1.4', '0.1.5')).toBe(false);
    expect(versionAtLeast('0.1', '0.1.5')).toBe(false);
  });
});
