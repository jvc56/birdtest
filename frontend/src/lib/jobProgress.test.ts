import { describe, expect, it } from 'vitest';
import type { JobStats } from './api';
import { jobProgress, rackProgress, taskCounts, throughputFigure, timeLeftFigure, when } from './jobProgress';

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
/** An opening-rack job's rack stats: by default a third of the way through a 3-to-7 consensus job's first pass. */
const racks = (over: Partial<NonNullable<JobStats['opening_racks']>> = {}): NonNullable<JobStats['opening_racks']> => ({
  racks_analyzed: 2060000,
  racks_settled: 0,
  racks_without_consensus: 0,
  racks_total: 3199724,
  first_pass_tasks_done: 4120,
  first_pass_tasks_total: 6400,
  reissue_tasks_done: 0,
  fewest_open_analyses: null,
  min_results_per_rack: 3,
  max_results_per_rack: 7,
  ...over
});
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
    const four = racks({ racks_settled: 1, racks_total: 4, racks_analyzed: 2 });
    expect(jobProgress(job({ opening_racks: four })).fraction).toBe(0.25);
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
      `about ${new Date(now + 960_000).toLocaleString(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`
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

describe('F-PROG-3 an opening-rack job\'s two fronts and its tasks', () => {
  const now = Date.parse('2026-10-13T15:00:00');
  const pace = { per_hour: 92000, unit: 'analysis' as const };

  it('gives racks analysed and racks settled each a Done and a Time left during the first pass', () => {
    const p = rackProgress(job({ opening_racks: racks(), throughput: pace, eta_seconds: 214400 }), now)!;
    expect(p.consensus).toBe(true);
    expect(p.headline).toBe('0.0% settled · 64.4% analysed');
    expect(p.rows.map((r) => [r.key, r.label, r.done])).toEqual([
      ['analysed', 'Analysed', 2060000],
      ['settled', 'Settled', 0]
    ]);
    // Every analysis goes to a new rack until the first pass ends.
    const eta = (3199724 - 2060000) / (92000 / 3600);
    expect(p.rows[0].left).toEqual({ value: '12.4h', sub: `about ${when(now + eta * 1000)}` });
    expect(p.rows[1].left).toEqual({ value: '2.5d', sub: `about ${when(now + 214400 * 1000)}` });
  });

  it('says every rack is analysed once the first pass is done, and finished when the job is', () => {
    const reanalysis = racks({ racks_analyzed: 3199724, racks_settled: 1144000, fewest_open_analyses: 2 });
    const p = rackProgress(job({ opening_racks: reanalysis, throughput: pace, eta_seconds: 80000 }), now)!;
    expect(p.rows[0].left).toEqual({ value: '—', sub: 'every rack analysed' });
    expect(p.headline).toBe('35.8% settled · 100.0% analysed');
    const done = racks({ racks_analyzed: 3199724, racks_settled: 3199724 });
    const completion = { at: '2026-10-12T23:40:00Z', forced: false, reason: null };
    const finished = rackProgress(job({ status: 'completed', opening_racks: done, completion }), now)!;
    expect(finished.headline).toBe('Done');
    expect(finished.rows[1].left.sub).toMatch(/^finished /);
  });

  it('keeps the idle wording while a job is off', () => {
    const p = rackProgress(job({ status: 'inactive', opening_racks: racks(), throughput: null }), now)!;
    expect(p.rows.map((r) => r.left)).toEqual([
      { value: '—', sub: 'paused while inactive' },
      { value: '—', sub: 'paused while inactive' }
    ]);
  });

  it('gives one row to a job analysing each rack once, which settles a rack as it analyses it', () => {
    const once = racks({ racks_settled: 2060000, min_results_per_rack: 1, max_results_per_rack: 1 });
    const p = rackProgress(job({ opening_racks: once, throughput: pace, eta_seconds: 44600 }), now)!;
    expect(p.consensus).toBe(false);
    expect(p.headline).toBe('64.4%');
    expect(p.rows.map((r) => r.label)).toEqual(['Analysed and settled']);
    expect(rackProgress(job({ games: pairs() }), now)).toBeNull();
  });

  it('splits an opening-rack job\'s done tasks into first pass and re-analysis', () => {
    const stats = job({ tasks_claimed: 40, tasks_available: 3, tasks_completed: 13120,
      opening_racks: racks({ first_pass_tasks_done: 6400, reissue_tasks_done: 6720 }) });
    expect(taskCounts(stats)).toEqual([
      { value: 6400, label: `of ${(6400).toLocaleString()} first pass` },
      { value: 6720, label: 're-analysis' },
      { value: 40, label: 'in progress' },
      { value: 3, label: 'to be handed out again' }
    ]);
    const once = job({ opening_racks: racks({ max_results_per_rack: 1 }) });
    expect(taskCounts(once).map((t) => t.label)).not.toContain('re-analysis');
    // Any other job: in flight, then done.
    expect(taskCounts(job({ tasks_claimed: 1, tasks_available: 2, tasks_completed: 3 }))).toEqual([
      { value: 1, label: 'in progress' },
      { value: 2, label: 'to be handed out again' },
      { value: 3, label: 'done' }
    ]);
  });
});
