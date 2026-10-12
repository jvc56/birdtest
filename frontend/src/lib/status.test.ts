import { describe, expect, it } from 'vitest';
import { backupStatus, fileStatus, jobStatus } from './status';

describe('F-STATUS-1 status pills', () => {
  it("gives a job's status its icon, completed grey rather than the links' blue", () => {
    expect(jobStatus('active')).toEqual({ word: 'active', tone: 'green', icon: 'play' });
    expect(jobStatus('inactive')).toEqual({ word: 'inactive', tone: 'grey', icon: 'pause' });
    expect(jobStatus('completed')).toEqual({ word: 'completed', tone: 'grey', icon: 'check' });
    // One the site does not know reads as itself; `constructor` is not a status.
    expect(jobStatus('set_aside')).toEqual({ word: 'set aside', tone: 'grey', icon: 'minus' });
    expect(jobStatus('constructor').icon).toBe('minus');
  });

  it("words a derived file's state as the pages do, and a backup's result", () => {
    expect(['pending', 'building', 'built', 'failed'].map((s) => fileStatus(s).word)).toEqual([
      'queued', 'building', 'built', 'failed'
    ]);
    expect(fileStatus('failed')).toMatchObject({ tone: 'red', icon: 'x' });
    expect(fileStatus('building').icon).toBe('spin');
    expect(backupStatus(true)).toEqual({ word: 'ok', tone: 'green', icon: 'check' });
    expect(backupStatus(false)).toMatchObject({ word: 'failed', tone: 'red' });
  });
});
