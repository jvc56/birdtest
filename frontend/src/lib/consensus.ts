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
