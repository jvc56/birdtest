/**
 * A job's Progress card: how far along it is, as a fraction and a percentage
 * over a bar, and its four figures -- what is done, its pace, the time left
 * and its allocation. On the public and admin job pages alike.
 *
 * Each job type counts its own work: a games or pairs job its games or pairs
 * against its cap, an opening-rack job its settled racks, a leave-generation
 * job its closed generations and the current one's share of racks at target,
 * so the bar moves through a generation rather than jumping when it closes.
 * A completed job is full: a job its significance test finished early is done,
 * not stuck at a few percent of a cap it no longer needs.
 */
import type { JobStats } from '$lib/api';
import { duration, throughputParts } from '$lib/format';

/** A figure under the bar: its value, and the line under it. */
export interface Figure {
  value: string;
  sub: string;
}

export interface JobProgress {
  /** How far along, 0 to 1; 1 once the job is completed. */
  fraction: number;
  done: boolean;
  /** "6.2%", or "Done". */
  headline: string;
  /** Why a completed job stopped short of its target, or null. */
  stopped: string | null;
  /** What is done, and of what. */
  work: Figure;
}

const pct = (f: number) => `${(f * 100).toFixed(1)}%`;
const fmt = (n: number) => n.toLocaleString();

/** How far along the job's own work is, before completion counts as full. */
function counted(stats: JobStats): { fraction: number; work: Figure } {
  const of = (done: number, total: number) => (total > 0 ? Math.min(1, done / total) : 0);
  const g = stats.games;
  if (g) {
    return {
      fraction: of(g.units_completed, g.max_units),
      work: { value: fmt(g.units_completed), sub: `of ${g.test ? 'up to ' : ''}${fmt(g.max_units)} ${g.unit}s` }
    };
  }
  const racks = stats.opening_racks;
  if (racks) {
    return {
      fraction: of(racks.racks_settled, racks.racks_total),
      work: { value: fmt(racks.racks_settled), sub: `of ${fmt(racks.racks_total)} racks settled` }
    };
  }
  const lg = stats.leave_generation;
  if (lg) {
    // The generation in play counts by its racks at target, until it closes.
    const current = lg.generations_closed < lg.generation_count ? of(lg.racks_at_target, lg.racks_total) : 0;
    return {
      fraction: of(lg.generations_closed + current, lg.generation_count),
      work: { value: `${lg.generations_closed} of ${lg.generation_count}`, sub: 'generations closed' }
    };
  }
  return {
    fraction: of(stats.tasks_completed, stats.tasks_total),
    work: { value: fmt(stats.tasks_completed), sub: `of ${fmt(stats.tasks_total)} tasks` }
  };
}

export function jobProgress(stats: JobStats): JobProgress {
  const { fraction, work } = counted(stats);
  const done = stats.job.status === 'completed';
  let stopped: string | null = null;
  if (done && fraction < 1) {
    const g = stats.games;
    const at = g ? `Stopped at ${fmt(g.units_completed)} of up to ${fmt(g.max_units)} ${g.unit}s` : `Stopped at ${pct(fraction)}`;
    if (stats.completion?.forced) stopped = `${at}: an admin completed it.`;
    else if (g?.decided) stopped = `${at}: the significance test decided at ${fmt(g.decided.units)}.`;
  }
  return { fraction: done ? 1 : fraction, done, headline: done ? 'Done' : pct(fraction), stopped, work };
}

/** A clock time a short way off: "Oct 14, 4:00 AM". */
const when = (ms: number) =>
  new Date(ms).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

/** Nothing moves while a job is off; a finished one has nothing left. */
function idle(stats: JobStats): string | null {
  if (stats.job.status === 'completed') return 'finished';
  if (stats.job.status === 'inactive') return 'paused while inactive';
  return null;
}

/** The job's pace in its own unit, the figure and its unit apart. */
export function throughputFigure(stats: JobStats): Figure {
  const parts = throughputParts(stats.throughput);
  return parts ? { value: parts.value, sub: parts.unit } : { value: '—', sub: idle(stats) ?? 'not measured yet' };
}

/** The time left at the current pace, and the clock time that reaches; `now` in ms. */
export function timeLeftFigure(stats: JobStats, now: number): Figure {
  if (stats.job.status === 'completed') {
    return { value: '—', sub: stats.completion ? `finished ${when(Date.parse(stats.completion.at))}` : 'finished' };
  }
  const eta = stats.eta_seconds;
  if (stats.job.status !== 'active' || eta === null || !isFinite(eta)) {
    return { value: '—', sub: idle(stats) ?? 'no estimate yet' };
  }
  return { value: duration(eta), sub: `about ${when(now + eta * 1000)}` };
}
