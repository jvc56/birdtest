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

/** A clock time, with its year so a long job's end reads unambiguously: "Oct 14, 2026, 4:00 AM". */
export const when = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit'
  });

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

/** One row of an opening-rack job's Progress table: its racks analysed, or settled, and the time left to all of them. */
export interface RackFront {
  key: 'analysed' | 'settled';
  label: string;
  done: number;
  total: number;
  fraction: number;
  left: Figure;
}

/** An opening-rack job's Progress card: the bar's two layers, the headline, and the table's rows. */
export interface RackProgress {
  /** Whether the job seeks a consensus: analyses a rack more than once. */
  consensus: boolean;
  analysed: number;
  settled: number;
  headline: string;
  rows: RackFront[];
}

/**
 * The rows of an opening-rack job's Progress table, and its headline; null
 * for another job type. With a consensus, racks analysed and racks settled
 * each have a Done and a Time left: re-analysis starts only once the first
 * pass covers the rack space, so until then every analysis goes to a new
 * rack, and the time to analyse them all is the racks not yet analysed at
 * the current pace. Settled keeps the server's estimate. A job analysing each
 * rack once settles a rack as it analyses it: one row. `now` in ms.
 */
export function rackProgress(stats: JobStats, now: number): RackProgress | null {
  const r = stats.opening_racks;
  if (!r) return null;
  const of = (n: number) => (r.racks_total > 0 ? Math.min(1, n / r.racks_total) : 0);
  const consensus = r.max_results_per_rack > 1;
  const finished = stats.job.status === 'completed';
  const settledLeft = timeLeftFigure(stats, now);
  const front = (key: RackFront['key'], label: string, done: number, left: Figure): RackFront => ({
    key,
    label,
    done,
    total: r.racks_total,
    fraction: of(done),
    left
  });
  if (!consensus) {
    const headline = finished ? 'Done' : pct(of(r.racks_settled));
    return {
      consensus,
      analysed: of(r.racks_analyzed),
      settled: of(r.racks_settled),
      headline,
      rows: [front('settled', 'Analysed and settled', r.racks_settled, settledLeft)]
    };
  }
  let analysedLeft: Figure;
  const perSecond = stats.throughput ? stats.throughput.per_hour / 3600 : 0;
  if (r.racks_analyzed >= r.racks_total || finished) analysedLeft = { value: '—', sub: 'every rack analysed' };
  else if (stats.job.status !== 'active' || !(perSecond > 0)) analysedLeft = { value: '—', sub: idle(stats) ?? 'no estimate yet' };
  else {
    const eta = (r.racks_total - r.racks_analyzed) / perSecond;
    analysedLeft = { value: duration(eta), sub: `about ${when(now + eta * 1000)}` };
  }
  return {
    consensus,
    analysed: of(r.racks_analyzed),
    settled: of(r.racks_settled),
    headline: finished ? 'Done' : `${pct(of(r.racks_settled))} settled · ${pct(of(r.racks_analyzed))} analysed`,
    rows: [
      front('analysed', 'Analysed', r.racks_analyzed, analysedLeft),
      front('settled', 'Settled', r.racks_settled, settledLeft)
    ]
  };
}

/**
 * The Progress card's tasks line, after "Tasks:". An opening-rack job's done
 * tasks split into first-pass ones, of the total that covers the rack space,
 * and -- seeking a consensus -- re-analysis ones; "to be handed out again" is
 * a task given back or timed out, waiting for its next claim.
 */
export function taskCounts(stats: JobStats): { value: number; label: string }[] {
  const r = stats.opening_racks;
  const inFlight = [
    { value: stats.tasks_claimed, label: 'in progress' },
    { value: stats.tasks_available, label: 'to be handed out again' }
  ];
  if (!r) return [...inFlight, { value: stats.tasks_completed, label: 'done' }];
  const done = [{ value: r.first_pass_tasks_done, label: `of ${fmt(r.first_pass_tasks_total)} first pass` }];
  if (r.max_results_per_rack > 1) done.push({ value: r.reissue_tasks_done, label: 're-analysis' });
  return [...done, ...inFlight];
}
