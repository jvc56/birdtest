import { describe, expect, it } from 'vitest';
import type { JobStats } from './api';
import { jobProgress, throughputFigure, timeLeftFigure } from './jobProgress';

/** A job's stats, with only what the Progress card reads. */
const job = (over: Partial<JobStats> & { status?: JobStats['job']['status'] }): JobStats =>
  ({
    job: { status: over.status ?? 'active', allocation: 20 },
    tasks_total: 0,
    tasks_completed: 0,
    tasks_available: 0,
    tasks_claimed: 0,
    throughput: null,
    eta_seconds: null,
    ...over
  }) as unknown as JobStats;
const pairs = (over: Record<string, unknown> = {}) =>
  ({ unit: 'pair', units_completed: 1240, max_units: 20000, min_units: 500, test: {}, ...over }) as unknown as JobStats['games'];

describe('F-PROG-1 jobProgress', () => {
  it('counts a games or pairs job against its cap, "up to" it when a test may stop it first', () => {
    const progress = jobProgress(job({ games: pairs() }));
    expect(progress).toMatchObject({ fraction: 0.062, done: false, headline: '6.2%', stopped: null });
    expect(progress.work).toEqual({ value: (1240).toLocaleString(), sub: `of up to ${(20000).toLocaleString()} pairs` });
    expect(jobProgress(job({ games: pairs({ test: null, units_completed: 24, max_units: 40 }) })).work.sub).toBe(
      'of 40 pairs'
    );
  });

  it('is full once completed, and says why a job stopped short of its cap', () => {
    const decided = jobProgress(
      job({ status: 'completed', games: pairs({ units_completed: 512, decided: { units: 500 } }) })
    );
    expect(decided).toMatchObject({ fraction: 1, done: true, headline: 'Done' });
    expect(decided.stopped).toBe(
      `Stopped at 512 of up to ${(20000).toLocaleString()} pairs: the significance test decided at 500.`
    );
    const forced = jobProgress(
      job({ status: 'completed', games: pairs(), completion: { at: '', forced: true, reason: null } })
    );
    expect(forced.stopped).toMatch(/: an admin completed it\.$/);
    // One that reached its cap has nothing to explain.
    expect(jobProgress(job({ status: 'completed', games: pairs({ units_completed: 20000 }) })).stopped).toBeNull();
  });

  it('counts an opening-rack job by its settled racks', () => {
    const racks = { racks_settled: 1, racks_total: 4, racks_analyzed: 2, racks_without_consensus: 0 };
    expect(jobProgress(job({ opening_racks: racks })).fraction).toBe(0.25);
  });

  it("counts a leave-generation job's current generation by its racks at target, so it moves between closings", () => {
    const lg = { generations_closed: 2, generation_count: 5, racks_at_target: 50, racks_total: 100 };
    const progress = jobProgress(job({ leave_generation: lg as unknown as JobStats['leave_generation'] }));
    expect(progress.fraction).toBeCloseTo(0.5);
    expect(progress.work).toEqual({ value: '2 of 5', sub: 'generations closed' });
    // Every generation closed: the last one's racks do not count twice.
    const closed = { ...lg, generations_closed: 5 };
    expect(jobProgress(job({ leave_generation: closed as unknown as JobStats['leave_generation'] })).fraction).toBe(1);
  });
});

describe('F-PROG-2 throughput and time left', () => {
  it('splits the pace into its figure and unit, and says why there is none', () => {
    expect(throughputFigure(job({ throughput: { per_hour: 340, unit: 'pair' } }))).toEqual({
      value: '340',
      sub: 'pairs/hour'
    });
    expect(throughputFigure(job({}))).toEqual({ value: '—', sub: 'not measured yet' });
    expect(throughputFigure(job({ status: 'inactive' })).sub).toBe('paused while inactive');
    expect(throughputFigure(job({ status: 'completed' })).sub).toBe('finished');
  });

  it('gives the time left and the clock time it reaches, only while the job is active', () => {
    const now = Date.parse('2026-10-11T20:40:00');
    const left = timeLeftFigure(job({ eta_seconds: 960 }), now);
    expect(left.value).toBe('16m');
    expect(left.sub).toBe(
      `about ${new Date(now + 960_000).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`
    );
    expect(timeLeftFigure(job({}), now)).toEqual({ value: '—', sub: 'no estimate yet' });
    expect(timeLeftFigure(job({ status: 'inactive', eta_seconds: 960 }), now)).toEqual({
      value: '—',
      sub: 'paused while inactive'
    });
    expect(timeLeftFigure(job({ status: 'completed', completion: { at: '2026-10-09T21:46:00Z', forced: false, reason: null } }), now).sub).toMatch(
      /^finished /
    );
  });
});
