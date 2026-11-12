/**
 * An opening-rack job's consensus: how many analyses a rack gets, and when
 * they agree on its best move.
 *
 * A job wanting one analysis per rack (`max_results_per_rack` 1) settles each
 * rack at its first. Otherwise a rack's first analysis ranks every play, and
 * each later one only the first's top `reissue_top_moves`. The rack settles
 * once its most common rank-1 move leads the runner-up by `win_margin`
 * analyses, the first counted, or once it has `max_results_per_rack`.
 */

export interface ConsensusSettings {
  max_results_per_rack: number;
  reissue_top_moves: number;
  win_margin: number;
}

/** The oldest MAGPIE that reads a reissue's candidates: a job that reissues racks needs it. */
export const CANDIDATES_MIN_MAGPIE = '0.1.5';

/** Whether `version` (`major.minor[.patch]`) is at least `floor`, compared as numbers. */
export function versionAtLeast(version: string, floor: string): boolean {
  const parts = (v: string) => v.trim().split('.').map((p) => Number(p));
  const [a, b] = [parts(version), parts(floor)];
  for (let i = 0; i < 3; i++) {
    const [x, y] = [a[i] ?? 0, b[i] ?? 0];
    if (x !== y) return x > y;
  }
  return true;
}

/** The consensus fields a request sends, as numbers. */
export function consensusFields(c: ConsensusSettings): ConsensusSettings {
  return {
    max_results_per_rack: Number(c.max_results_per_rack),
    reissue_top_moves: Number(c.reissue_top_moves),
    win_margin: Number(c.win_margin)
  };
}

/**
 * What is wrong with the fields `consensusFields` sends, as the server's
 * `consensus_problems` would refuse them, or null: checked before a job is
 * created with them and before they are changed on one.
 */
export function consensusProblem(c: ConsensusSettings): string | null {
  const { max_results_per_rack: max, reissue_top_moves: top, win_margin: margin } = consensusFields(c);
  if (!(Number.isInteger(max) && max >= 1 && max <= 100)) return 'The most analyses must be a whole number from 1 to 100.';
  if (!(Number.isInteger(top) && top >= 2 && top <= 100)) return 'The moves reanalysed must be a whole number from 2 to 100.';
  if (!(Number.isInteger(margin) && margin >= 1 && margin <= 100)) return 'The lead must be a whole number from 1 to 100.';
  return null;
}

/** The fewest analyses that can settle a rack: its first, and as many more as the lead needs. */
export function fewestToSettle(c: ConsensusSettings): number {
  return c.max_results_per_rack > 1 ? Math.min(c.win_margin, c.max_results_per_rack) : 1;
}

/** The settings as the job's settings table states them. */
export function analysesPerRack(c: ConsensusSettings): string {
  if (c.max_results_per_rack <= 1) return '1';
  return (
    `up to ${c.max_results_per_rack}, until one move leads by ${c.win_margin}; ` +
    `later analyses rank the first's top ${c.reissue_top_moves} moves`
  );
}

/** One ranked move of a rack lookup: `analysis` numbers the rack's analyses from 1. */
export interface LookupMove {
  analysis?: number;
  rank: number;
  move: string;
}

export interface RackConsensus {
  analyses: number;
  /** The most common best move, the alphabetically first of a tie. */
  top: string;
  /** How many analyses ranked it first. */
  count: number;
  /** How many ranked the runner-up first: the next most common best move's count, 0 for none. */
  runnerUp: number;
  /** `count` as a share of `analyses`, to one decimal place: "80.0". */
  share: string;
}

/** What a rack's analyses agree on, from its lookup; null for no analyses. */
export function rackConsensus(moves: LookupMove[]): RackConsensus | null {
  const firsts = new Map<string, number>();
  let analyses = 0;
  for (const m of moves) {
    if (m.rank !== 1) continue;
    analyses += 1;
    firsts.set(m.move, (firsts.get(m.move) ?? 0) + 1);
  }
  if (!analyses) return null;
  const ranked = [...firsts.entries()].sort(([a, x], [b, y]) => y - x || (a < b ? -1 : a > b ? 1 : 0));
  const [top, count] = ranked[0];
  const runnerUp = ranked[1]?.[1] ?? 0;
  return { analyses, top, count, runnerUp, share: ((100 * count) / analyses).toFixed(1) };
}

/** An English ordinal: 1st, 2nd, 3rd, 4th, 11th, 21st. */
export function ordinal(n: number): string {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  return `${n}${({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[n % 10] ?? 'th'}`;
}

/** What the Opening racks card's stage sentence reads from the job's stats. */
export interface RackStage {
  racks_analyzed: number;
  racks_settled: number;
  racks_without_consensus: number;
  racks_total: number;
  fewest_open_analyses: number | null;
  max_results_per_rack: number;
  win_margin: number;
  reissue_top_moves: number;
}

/**
 * Where a job seeking a consensus stands, as the Opening racks card says it:
 * its first pass (no rack can settle before the lead it needs), its
 * re-analysis (open racks are analysed fewest first), or finished. `null` for
 * a job analysing each rack once, which has no consensus to stand anywhere.
 */
export function consensusStage(s: RackStage, finished: boolean): { stage: string; text: string } | null {
  if (s.max_results_per_rack <= 1) return null;
  const pct = (n: number) => `${((100 * n) / Math.max(1, s.racks_total)).toFixed(1)}%`;
  const count = (n: number) => n.toLocaleString();
  if (finished || (s.racks_total > 0 && s.racks_settled >= s.racks_total)) {
    const agreed = s.racks_settled - s.racks_without_consensus;
    return {
      stage: 'Finished.',
      text: `Every rack is settled: ${count(agreed)} agreed, ${count(s.racks_without_consensus)} without a consensus after ${s.max_results_per_rack} analyses.`
    };
  }
  if (s.racks_analyzed < s.racks_total) {
    const fewest = fewestToSettle(s);
    const wait = fewest > 1 ? ` None can settle before its ${ordinal(fewest)}.` : '';
    return {
      stage: 'First pass.',
      text: `${pct(s.racks_analyzed)} of racks have their first analysis.${wait} Re-analysis starts when every rack has one.`
    };
  }
  const fewest = s.fewest_open_analyses ?? 1;
  return {
    stage: 'Re-analysis.',
    text: `Every open rack has at least ${fewest} ${fewest === 1 ? 'analysis' : 'analyses'}, and racks with the fewest are analysed first, over their first analysis's top ${s.reissue_top_moves} moves. A rack settles when one move leads by ${s.win_margin}.`
  };
}
