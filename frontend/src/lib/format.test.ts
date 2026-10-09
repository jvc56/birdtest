import { describe, expect, it } from 'vitest';
import {
  blankFields,
  computeTime,
  exactCount,
  unchosenText,
  datetime,
  derivedKind,
  duration,
  jobTypeLabel,
  leavePlayerConflict,
  optionalNumber,
  optionalIntList,
  parseTargetRackCounts,
  targetsText,
  MAX_LEAVE_GENERATIONS,
  MAX_TARGET_RACK_COUNT,
  testLabel,
  testState,
  scorePct,
  completionText,
  exportSummary,
  jobTitle,
  workerLabel
} from './format';

describe('F-FMT-1 workerLabel', () => {
  const uuid = '3f2b8c1e-9d4a-4e7b-a1c2-5d6e7f809a1b';

  it('renders the username when there is one', () => {
    expect(workerLabel({ username: 'alice', anon_id: uuid })).toBe('alice');
  });

  it('renders "Anonymous" and a short prefix when there is no username', () => {
    expect(workerLabel({ username: null, anon_id: uuid })).toBe('Anonymous · 3f2b8c1e');
    expect(workerLabel({ anon_id: uuid })).toBe('Anonymous · 3f2b8c1e');
  });

  it('never leaks the full identifier', () => {
    const label = workerLabel({ username: null, anon_id: uuid });
    expect(label).not.toContain(uuid);
    // Nothing past the first eight characters appears at all.
    expect(label).not.toContain(uuid.slice(8));
    expect(label).not.toContain('-');
  });

  it('renders a pseudonym whole, as `?worker=` takes it', () => {
    expect(workerLabel({ anon_id: '3f2b8c1e9d4a4e7b' })).toBe('Anonymous · 3f2b8c1e9d4a4e7b');
  });

  it('treats an empty username as absent', () => {
    expect(workerLabel({ username: '', anon_id: uuid })).toBe('Anonymous · 3f2b8c1e');
  });

  it('falls back to "Unknown" with neither', () => {
    expect(workerLabel({})).toBe('Unknown');
    expect(workerLabel({ username: null, anon_id: null })).toBe('Unknown');
  });
});

describe('F-FMT-2 duration', () => {
  it('renders null and non-finite values as a dash', () => {
    expect(duration(null)).toBe('—');
    expect(duration(NaN)).toBe('—');
    expect(duration(Infinity)).toBe('—');
    expect(duration(null)).not.toContain('null');
  });

  it('renders seconds below a minute', () => {
    expect(duration(0)).toBe('0s');
    expect(duration(1)).toBe('1s');
    expect(duration(59)).toBe('59s');
    expect(duration(59.4)).toBe('59s');
  });

  it('switches to minutes at the minute boundary', () => {
    expect(duration(60)).toBe('1m');
    // Would round to "60s" if the unit were chosen before rounding.
    expect(duration(59.6)).toBe('1m');
    expect(duration(90)).toBe('2m');
    expect(duration(3569)).toBe('59m');
  });

  it('switches to hours at the hour boundary', () => {
    expect(duration(3600)).toBe('1.0h');
    // Would read "60m" if the unit were chosen before rounding.
    expect(duration(3599)).toBe('1.0h');
    expect(duration(5400)).toBe('1.5h');
    expect(duration(86_000)).toBe('23.9h');
  });

  it('switches to days at the day boundary', () => {
    expect(duration(86_400)).toBe('1.0d');
    // Would read "24.0h" if the unit were chosen before rounding.
    expect(duration(86_399)).toBe('1.0d');
    expect(duration(3 * 86_400 + 43_200)).toBe('3.5d');
  });
});

describe('F-FMT-3 datetime', () => {
  it('renders null as a dash', () => {
    expect(datetime(null)).toBe('—');
    expect(datetime('')).toBe('—');
  });

  it('renders a valid ISO string as a local time', () => {
    const iso = '2026-03-04T05:06:07Z';
    expect(datetime(iso)).toBe(new Date(iso).toLocaleString());
    expect(datetime(iso)).not.toBe(iso);
  });

  it('does not throw on an unparseable string', () => {
    expect(() => datetime('not a date')).not.toThrow();
    expect(typeof datetime('not a date')).toBe('string');
  });
});

describe('F-FMT-4 jobTypeLabel', () => {
  it('covers all four job types', () => {
    expect(jobTypeLabel('opening_rack')).toBe('Opening Rack Analysis');
    expect(jobTypeLabel('games')).toBe('Games');
    expect(jobTypeLabel('game_pairs')).toBe('Game Pairs');
    expect(jobTypeLabel('leave_generation')).toBe('Leave Generation');
  });

  it('falls back to the raw string for an unknown type', () => {
    expect(jobTypeLabel('something_new')).toBe('something_new');
    expect(jobTypeLabel('something_new')).not.toContain('undefined');
    // Not fooled by Object.prototype members.
    expect(jobTypeLabel('toString')).toBe('toString');
  });
});

describe('derivedKind', () => {
  it('names each derived file the server builds', () => {
    expect(derivedKind('wmp')).toBe('Wordmap');
    expect(derivedKind('rit')).toBe('Rack info table');
    // Once every role but `wmp` read as a rack info table.
    expect(derivedKind('wit')).toBe('Word info table');
    expect(derivedKind('other')).toBe('other');
  });
});

describe('F-FMT-5b testState', () => {
  const running = { test: { status: 'running' } };
  it("says paused, not running, while the job is inactive", () => {
    expect(testState('inactive', running)).toBe('paused');
    expect(testLabel(testState('inactive', running))).toBe('paused while the job is inactive');
  });
  it('is the test while the job is active', () => {
    expect(testState('active', running)).toBe('running');
    expect(testState('active', { test: { status: 'player1_better' } })).toBe('player1_better');
  });
  it('is the decision a completed job stopped on, or undecided without one', () => {
    expect(testState('completed', { ...running, decided: { status: 'inconclusive' } })).toBe('inconclusive');
    expect(testState('completed', running)).toBe('undecided');
    // A purged job that had a decision keeps none: the purge clears it.
    expect(testState('inactive', { test: { status: 'player2_better' } })).toBe('paused');
  });
  it('is off, whatever the job is doing, for a job that runs no test', () => {
    for (const status of ['active', 'inactive', 'completed']) {
      expect(testState(status, { test: null })).toBe('off');
    }
    expect(testLabel('off')).toBe('not run: the job plays to its target');
  });
});

describe('F-FMT-5 testLabel', () => {
  it('covers every status', () => {
    expect(testLabel('running')).toBe('running');
    expect(testLabel('player1_better')).toBe('decided: player 1 is better');
    expect(testLabel('player2_better')).toBe('decided: player 2 is better');
    expect(testLabel('inconclusive')).toBe('inconclusive: the job reached its cap first');
  });

  it('falls back to the raw status for an unknown one', () => {
    expect(testLabel('abandoned')).toBe('abandoned');
    expect(testLabel('constructor')).toBe('constructor');
  });
});

describe('F-FMT-5c scores', () => {
  it('shows a score per game as a percentage to a tenth', () => {
    expect(scorePct(0.53125)).toBe('53.1%');
    expect(scorePct(0.5)).toBe('50.0%');
    expect(scorePct(1)).toBe('100.0%');
  });
});

describe('F-FMT-6 form numbers', () => {
  it('reads a blank optional number as null, never 0', () => {
    // Svelte binds a cleared number box as null; Number(null) is 0.
    expect(optionalNumber(null)).toBeNull();
    expect(optionalNumber('')).toBeNull();
    expect(optionalNumber(undefined)).toBeNull();
    expect(optionalNumber(Number.NaN)).toBeNull();
    expect(optionalNumber(0)).toBe(0);
    expect(optionalNumber(2.5)).toBe(2.5);
    expect(optionalNumber('7')).toBe(7);
  });

  it('reads a blank list as the default and refuses a part that is not a whole number', () => {
    expect(optionalIntList('')).toEqual({ values: null });
    expect(optionalIntList('  ')).toEqual({ values: null });
    expect(optionalIntList('32, 16,8 ,4,2')).toEqual({ values: [32, 16, 8, 4, 2] });
    expect(optionalIntList('7')).toEqual({ values: [7] });
    expect(optionalIntList('8, x')).toEqual({ error: '"x" is not a whole number.' });
    expect(optionalIntList('8,,4')).toEqual({ error: '"" is not a whole number.' });
    expect(optionalIntList('-2')).toEqual({ error: '"-2" is not a whole number.' });
  });

  it('names the fields a request would send blank', () => {
    expect(blankFields({ a: 1, b: null, c: Number.NaN, d: 'x', e: 0 })).toEqual(['b', 'c']);
    expect(blankFields({ a: 1 })).toEqual([]);
  });

  it('names the selects left on their empty choice', () => {
    const choose = (ld: string, layout: string) =>
      unchosenText({ 'a letter distribution': ld, 'a board layout': layout });
    expect(choose('', '')).toBe('Choose a letter distribution and a board layout.');
    expect(choose('x', '')).toBe('Choose a board layout.');
    expect(choose('', 'y')).toBe('Choose a letter distribution.');
    expect(choose('x', 'y')).toBeNull();
    expect(unchosenText({ a: '', b: '', c: '' })).toBe('Choose a, b and c.');
  });
});

describe('F-FMT-12 completionText', () => {
  const games = (decided?: { status: string; lower: number; upper: number; units: number }) => ({
    unit: 'pair',
    max_units: 5000,
    test: { confidence_pct: 95 } as { confidence_pct: number } | null,
    decided
  });
  const untested = { ...games(), test: null };
  const pairs = { job_type: 'game_pairs' };
  it('tells a test that decided from a cap that was reached', () => {
    expect(
      completionText({
        job: pairs,
        completion: { forced: false, reason: 'player1_better' },
        games: games({ status: 'player1_better', lower: 0.5012, upper: 0.5523, units: 1200 })
      })
    ).toBe(
      'its significance test found player 1 better at 95% confidence after 1,200 pairs: player 1 scored 50.1% to 55.2% per game'
    );
    expect(
      completionText({
        job: pairs,
        completion: { forced: false, reason: 'player2_better' },
        games: games({ status: 'player2_better', lower: 0.41, upper: 0.4987, units: 900 })
      })
    ).toBe(
      'its significance test found player 2 better at 95% confidence after 900 pairs: player 1 scored 41.0% to 49.9% per game'
    );
    expect(
      completionText({
        job: pairs,
        completion: { forced: false, reason: 'inconclusive' },
        games: games({ status: 'inconclusive', lower: 0.495, upper: 0.507, units: 5000 })
      })
    ).toBe(
      'it reached its cap of 5,000 pairs before its significance test decided: player 1 scored 49.5% to 50.7% per game, at 95% confidence'
    );
  });
  it('says when an admin forced it', () => {
    expect(completionText({ job: pairs, completion: { forced: true, reason: null }, games: games() })).toBe(
      'an admin force-completed it before its test decided'
    );
    // With no test there is nothing it came before.
    expect(completionText({ job: pairs, completion: { forced: true, reason: null }, games: untested })).toBe(
      'an admin force-completed it'
    );
  });
  it('says a job without a test played what it was set to', () => {
    expect(
      completionText({ job: pairs, completion: { forced: false, reason: 'reached_target' }, games: untested })
    ).toBe('it played the 5,000 pairs it was set to');
  });
  it('names the other job types\' own ends', () => {
    expect(
      completionText({ job: { job_type: 'opening_rack' }, completion: { forced: false, reason: null } })
    ).toBe('every rack was settled');
    expect(
      completionText({
        job: { job_type: 'leave_generation' },
        completion: { forced: false, reason: 'last generation built' }
      })
    ).toBe('its last generation was built');
  });
});

describe('F-FMT-13 jobTitle', () => {
  it("is the job's name, or its type for one given none", () => {
    expect(jobTitle({ name: 'equity vs static', job_type: 'game_pairs' })).toBe('equity vs static');
    expect(jobTitle({ name: '', job_type: 'game_pairs' })).toBe('Game Pairs');
    expect(jobTitle({ name: '   ', job_type: 'games' })).toBe(jobTitle({ name: '', job_type: 'games' }));
  });
});

describe('F-FMT-14 exportSummary', () => {
  const at = '2026-09-29T12:00:00Z';
  const running = { status: 'active', job_type: 'games' };
  const done = { status: 'completed', job_type: 'games' };
  const snapshot = { state: 'ready', is_final: false, snapshot_at: at };
  const final = { state: 'ready', is_final: true, snapshot_at: at };

  it('labels a snapshot of a running job with its time', () => {
    const summary = exportSummary(snapshot, running);
    expect(summary.label).toBe(`Snapshot as of ${datetime(at)} — job still running`);
    expect(summary.button).toBe('Export a new snapshot');
    expect(summary.note).toBeNull();
    expect(exportSummary(null, running).button).toBe('Export a snapshot');
    expect(exportSummary(null, { status: 'inactive', job_type: 'games' }).button).toBe(
      'Export a snapshot'
    );
  });

  it("offers a completed job whose newest export is a snapshot its final one", () => {
    const summary = exportSummary(snapshot, done);
    expect(summary.label).toBe(
      `Snapshot as of ${datetime(at)}, read before the job completed — not its final results`
    );
    expect(summary.button).toBe('Build the final export');
    expect(exportSummary({ ...snapshot, state: 'expired' }, done).button).toBe(
      'Build the final export'
    );
  });

  it("does not say a completed opening-rack job's demoted export was read while it ran", () => {
    // A consensus edit demotes the final export of a job it leaves completed:
    // read after the job completed, but under the old settings.
    const opening = { status: 'completed', job_type: 'opening_rack' };
    const summary = exportSummary(snapshot, opening);
    expect(summary.label).toBe(
      `Snapshot as of ${datetime(at)} — not the job's final results: read before it completed, ` +
        'or before its consensus settings last changed'
    );
    expect(summary.label).not.toContain('running');
    expect(summary.button).toBe('Build the final export');
  });

  it("labels a completed job's final export, and offers it again", () => {
    expect(exportSummary(final, done)).toEqual({
      label: 'Final results',
      note: null,
      button: 'Export again'
    });
    expect(exportSummary(null, done).button).toBe('Export results');
    // A failed attempt at the final export is retried as one.
    const failed = { state: 'failed', is_final: false, snapshot_at: null };
    expect(exportSummary(failed, done)).toEqual({
      label: null,
      note: null,
      button: 'Build the final export'
    });
    // Nothing is labelled while it builds.
    expect(exportSummary({ state: 'running', is_final: false, snapshot_at: null }, done).label).toBeNull();
  });

  it("says a running leave job's snapshot is as of its last merge", () => {
    const leave = { status: 'active', job_type: 'leave_generation' };
    expect(exportSummary(snapshot, leave).note).toContain('last merge');
    expect(exportSummary(final, { ...leave, status: 'completed' }).note).toBeNull();
  });
});

describe('F-FMT-15 parseTargetRackCounts', () => {
  it("reads a comma-separated list as MAGPIE's leavegen takes it", () => {
    expect(parseTargetRackCounts('100,200,500,1000')).toEqual({ targets: [100, 200, 500, 1000] });
    expect(parseTargetRackCounts(' 100 , 200, 500, ')).toEqual({ targets: [100, 200, 500] });
    expect(parseTargetRackCounts('500')).toEqual({ targets: [500] });
    expect(parseTargetRackCounts('100 200  500')).toEqual({ targets: [100, 200, 500] });
  });

  it('refuses a thousands separator, which reads as two targets', () => {
    expect(parseTargetRackCounts('100, 1,000')).toEqual({
      error: '"1,000" reads as two targets: write 1000 without a thousands separator.'
    });
  });

  it('names what is wrong rather than sending it', () => {
    for (const bad of ['', '  ', ',', '100,,200', '100, x', '1.5', '-3', '0', '100, 0']) {
      expect(parseTargetRackCounts(bad), bad).toHaveProperty('error');
    }
    expect(parseTargetRackCounts('100, abc')).toEqual({
      error: '"abc" is not a whole number of occurrences.'
    });
    expect(parseTargetRackCounts(String(MAX_TARGET_RACK_COUNT))).toEqual({
      targets: [MAX_TARGET_RACK_COUNT]
    });
    expect(parseTargetRackCounts(String(MAX_TARGET_RACK_COUNT + 1))).toHaveProperty('error');
    const most = Array(MAX_LEAVE_GENERATIONS).fill('10');
    expect(parseTargetRackCounts(most.join(','))).toHaveProperty('targets');
    expect(parseTargetRackCounts([...most, '10'].join(','))).toEqual({
      error: `At most ${MAX_LEAVE_GENERATIONS} generations, not ${MAX_LEAVE_GENERATIONS + 1}.`
    });
  });
});

describe('F-FMT-17 leavePlayerConflict', () => {
  const player = { name: 'static', num_plies: 0, sort_strategy: 'equity', use_rit: false };

  it('accepts a static player that sorts on equity with no rack info table', () => {
    expect(leavePlayerConflict(player)).toBeNull();
  });

  it('names everything job creation would refuse', () => {
    expect(leavePlayerConflict({ ...player, num_plies: 1 })).toBe(
      'static simulates 1 ply; leave generation plays statically on equity, without a rack info table ' +
        'or endgame solving.'
    );
    expect(leavePlayerConflict({ ...player, num_plies: 2, sort_strategy: 'score', use_rit: true })).toBe(
      'static simulates 2 plies, sorts on score and asks for a rack info table; leave generation ' +
        'plays statically on equity, without a rack info table or endgame solving.'
    );
    expect(leavePlayerConflict({ ...player, endgame_plies: 6 })).toBe(
      'static solves endgames; leave generation plays statically on equity, without a rack info ' +
        'table or endgame solving.'
    );
    expect(leavePlayerConflict({ ...player, endgame_plies: 0 })).toBeNull();
  });
});

describe('F-FMT-16 computeTime', () => {
  it('reads a total to the second, in every unit that is not zero', () => {
    expect(computeTime(0)).toBe('0s');
    expect(computeTime(45)).toBe('45s');
    expect(computeTime(59.9)).toBe('59s');
    expect(computeTime(60)).toBe('1m');
    expect(computeTime(133)).toBe('2m 13s');
    expect(computeTime(3599)).toBe('59m 59s');
    expect(computeTime(3600)).toBe('1h');
    expect(computeTime(3601)).toBe('1h 1s');
    expect(computeTime(5 * 3600 + 20 * 60 + 13)).toBe('5h 20m 13s');
    expect(computeTime(86400)).toBe('1d');
    expect(computeTime(3 * 86400 + 4 * 3600 + 5 * 60 + 6)).toBe('3d 4h 5m 6s');
    expect(computeTime(3 * 86400 + 59)).toBe('3d 59s');
    expect(computeTime(365 * 86400)).toBe('1y');
    expect(computeTime((2 * 365 + 17) * 86400 + 3600)).toBe('2y 17d 1h');
    expect(computeTime(1234 * 365 * 86400)).toBe('1,234y');
  });

  it('shows nothing it cannot read as a time', () => {
    expect(computeTime(null)).toBe('—');
    expect(computeTime(Number.NaN)).toBe('—');
    expect(computeTime(Infinity)).toBe('—');
    expect(computeTime(-1)).toBe('—');
  });
});

describe('F-FMT-19 exactCount', () => {
  it('reads a count to its last digit, grouped', () => {
    expect(exactCount(0)).toBe('0');
    expect(exactCount(999)).toBe('999');
    expect(exactCount(1000)).toBe('1,000');
    expect(exactCount(1_234_567_890)).toBe('1,234,567,890');
    expect(exactCount(12.9)).toBe('12');
  });

  it('shows nothing it cannot read as a count', () => {
    expect(exactCount(null)).toBe('—');
    expect(exactCount(Number.NaN)).toBe('—');
    expect(exactCount(-1)).toBe('—');
  });
});

describe('F-FMT-18 targetsText', () => {
  it('joins the targets in generation order with arrows, not commas', () => {
    expect(targetsText([100, 1000, 1000])).toBe(`100 → ${(1000).toLocaleString()} → ${(1000).toLocaleString()}`);
    expect(targetsText([500])).toBe('500');
  });
});
