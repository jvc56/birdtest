/**
 * An opening-rack job's consensus: how many analyses a rack gets, and how
 * much they agree on its best move.
 *
 * A job wanting one analysis per rack (`max_results_per_rack` 1) settles each
 * rack at its first. Otherwise a rack is analysed until at least
 * `min_results_per_rack` analyses agree on its best move in `consensus_pct`
 * percent of them, or until it has `max_results_per_rack` analyses.
 */

export interface ConsensusSettings {
  consensus_pct: number;
  min_results_per_rack: number;
  max_results_per_rack: number;
}

/** The consensus fields a request sends: the share only past one analysis per rack. */
export interface ConsensusFields {
  min_results_per_rack: number;
  max_results_per_rack: number;
  consensus_pct?: number;
}

/**
 * The consensus fields a request sends, from the settings typed: the share
 * only when a rack can have more than one analysis. One analysis per rack
 * seeks no agreement, so its share box is disabled, and whatever it holds is
 * left out rather than refused: the server then checks the share it already
 * has, its default 100% at creation or the job's stored one on a change.
 */
export function consensusFields(c: ConsensusSettings): ConsensusFields {
  const min = Number(c.min_results_per_rack);
  const max = Number(c.max_results_per_rack);
  return {
    min_results_per_rack: min,
    max_results_per_rack: max,
    ...(max > 1 ? { consensus_pct: Number(c.consensus_pct) } : {})
  };
}

/**
 * What is wrong with the fields `consensusFields` sends, as the server's
 * `consensus_problems` would refuse them, or null: checked before a job is
 * created with them and before they are changed on one. The fewest and most
 * are checked whatever the most; the share only when it is sent.
 */
export function consensusProblem(c: ConsensusSettings): string | null {
  const { min_results_per_rack: min, max_results_per_rack: max, consensus_pct: pct } =
    consensusFields(c);
  if (!(Number.isInteger(min) && min >= 1 && min <= 100)) return 'The fewest analyses must be a whole number from 1 to 100.';
  if (!(Number.isInteger(max) && max >= min && max <= 100)) return 'The most analyses must be at least the fewest, and at most 100.';
  if (pct !== undefined && !(pct > 50 && pct <= 100)) return 'The share that must agree must be above 50% and at most 100%.';
  return null;
}

/** The settings as the job's settings table states them. */
export function analysesPerRack(c: ConsensusSettings): string {
  if (c.max_results_per_rack <= 1) return '1';
  const range =
    c.min_results_per_rack === c.max_results_per_rack
      ? `${c.min_results_per_rack}`
      : `${c.min_results_per_rack} to ${c.max_results_per_rack}`;
  return `${range}, until ${c.consensus_pct}% agree on the best move`;
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
  const [top, count] = [...firsts.entries()].sort(
    ([a, x], [b, y]) => y - x || (a < b ? -1 : a > b ? 1 : 0)
  )[0];
  return { analyses, top, count, share: ((100 * count) / analyses).toFixed(1) };
}

/** An English ordinal: 1st, 2nd, 3rd, 4th, 11th, 21st. */
function ordinal(n: number): string {
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
  min_results_per_rack: number;
  max_results_per_rack: number;
}

/**
 * Where a job seeking a consensus stands, as the Opening racks card says it:
 * its first pass (no rack can settle before its fewest analyses), its
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
    const wait =
      s.min_results_per_rack > 1 ? ` None can settle before its ${ordinal(s.min_results_per_rack)}.` : '';
    return {
      stage: 'First pass.',
      text: `${pct(s.racks_analyzed)} of racks have their first analysis.${wait} Re-analysis starts when every rack has one.`
    };
  }
  const fewest = s.fewest_open_analyses ?? 1;
  return {
    stage: 'Re-analysis.',
    text: `Every open rack has at least ${fewest} ${fewest === 1 ? 'analysis' : 'analyses'}, and racks with the fewest are analysed first.`
  };
}
