import { describe, expect, it } from 'vitest';
import {
  blankFields,
  datetime,
  duration,
  jobTypeLabel,
  optionalNumber,
  sprtLabel,
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
    expect(jobTypeLabel('opening_rack')).toBe('Opening rack analysis');
    expect(jobTypeLabel('games')).toBe('Games');
    expect(jobTypeLabel('game_pairs')).toBe('Game pairs');
    expect(jobTypeLabel('leave_generation')).toBe('Leave generation');
  });

  it('falls back to the raw string for an unknown type', () => {
    expect(jobTypeLabel('something_new')).toBe('something_new');
    expect(jobTypeLabel('something_new')).not.toContain('undefined');
    // Not fooled by Object.prototype members.
    expect(jobTypeLabel('toString')).toBe('toString');
  });
});

describe('F-FMT-5 sprtLabel', () => {
  it('covers all four statuses', () => {
    expect(sprtLabel('running')).toBe('running');
    expect(sprtLabel('passed')).toBe('passed (H1 accepted)');
    expect(sprtLabel('failed')).toBe('failed (H0 accepted)');
    expect(sprtLabel('terminated_at_max')).toBe('terminated at max games');
  });

  it('falls back to the raw status for an unknown one', () => {
    expect(sprtLabel('paused')).toBe('paused');
    expect(sprtLabel('constructor')).toBe('constructor');
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

  it('names the fields a request would send blank', () => {
    expect(blankFields({ a: 1, b: null, c: Number.NaN, d: 'x', e: 0 })).toEqual(['b', 'c']);
    expect(blankFields({ a: 1 })).toEqual([]);
  });
});
