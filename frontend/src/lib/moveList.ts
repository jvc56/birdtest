/**
 * What a move list shows beside each move when it was simulated -- its win
 * percentage and its first plies' statistics -- and, for an in-game position,
 * what was inferred of the opponent's leave first.
 */
import type { Inference, PlyStats } from '$lib/api';

/** Whether any of these moves was simulated, so has iterations to show. */
export function showsIterations(moves: { iterations: number | null }[]): boolean {
  return moves.some((m) => (m.iterations ?? 0) > 0);
}

/** The most plies a move list shows: P1 and P2. */
export const SHOWN_PLIES = 2;

/**
 * How many plies of statistics to show for these moves: the most any of them
 * has, up to two -- fewer when the job recorded fewer, none when nothing was
 * simulated.
 */
export function plyColumns(moves: { plies: PlyStats[] }[]): number {
  return Math.min(SHOWN_PLIES, Math.max(0, ...moves.map((m) => m.plies.length)));
}

/** Ply `index`'s statistics of a move (P1 is index 0), or null when it has none. */
export function plyAt(plies: PlyStats[], index: number): PlyStats | null {
  return plies.find((p) => p.ply === index) ?? null;
}

/** The column headers for `count` plies: P1-S, P1-BP, P2-S, P2-BP. */
export function plyHeaders(count: number): { label: string; title: string }[] {
  const headers: { label: string; title: string }[] = [];
  for (let i = 1; i <= count; i++) {
    const whose = i === 1 ? "the reply's" : `ply ${i}'s`;
    headers.push({ label: `P${i}-S`, title: `Ply ${i}: ${whose} average score` });
    headers.push({ label: `P${i}-BP`, title: `Ply ${i}: ${whose} bingo percentage` });
  }
  return headers;
}

/** An inference, in a line: how many leaves it found, from what, and their mean equity. */
export function inferenceSummary(inference: Inference, previousMove: string | null): string {
  const from = previousMove ? ` from ${previousMove}` : '';
  const leaves = `${inference.num_leaves.toLocaleString()} possible leave${inference.num_leaves === 1 ? '' : 's'}`;
  return `Inferred${from}: ${leaves}, average equity ${inference.average_equity.toFixed(1)}`;
}

/** A leave's draws as a share of all the inference drew, to a tenth: "12.5%". */
export function drawShare(draws: number, total: number): string {
  return total > 0 ? `${((100 * draws) / total).toFixed(1)}%` : '—';
}
