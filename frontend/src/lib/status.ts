/**
 * The site's status pills: each state's word, its tone, and the icon that
 * says it by shape as well as colour -- play for a job running, pause for one
 * off, a tick for one done, a spinning arc for work under way, a clock for
 * work waiting, a cross for a failure, a warning for something stuck. Done
 * is grey, not blue: blue is the links' colour and player 1's.
 */

export type StatusTone = 'green' | 'amber' | 'red' | 'grey' | 'blue' | 'player1' | 'player2';
export type StatusIcon = 'play' | 'pause' | 'check' | 'warn' | 'spark' | 'spin' | 'minus' | 'clock' | 'x';

export interface Status {
  word: string;
  tone: StatusTone;
  icon: StatusIcon;
}

const JOB: Record<string, Omit<Status, 'word'>> = {
  active: { tone: 'green', icon: 'play' },
  inactive: { tone: 'grey', icon: 'pause' },
  completed: { tone: 'grey', icon: 'check' }
};

/** A job's status. One the site does not know reads as itself, in grey. */
export function jobStatus(status: string): Status {
  const known = Object.prototype.hasOwnProperty.call(JOB, status) ? JOB[status] : null;
  return { word: status.replace(/_/g, ' '), ...(known ?? { tone: 'grey', icon: 'minus' }) };
}

const FILE: Record<string, Status> = {
  pending: { word: 'queued', tone: 'grey', icon: 'clock' },
  building: { word: 'building', tone: 'amber', icon: 'spin' },
  built: { word: 'built', tone: 'green', icon: 'check' },
  failed: { word: 'failed', tone: 'red', icon: 'x' }
};

/** A derived data file's state. */
export function fileStatus(state: string): Status {
  return Object.prototype.hasOwnProperty.call(FILE, state)
    ? FILE[state]
    : { word: state, tone: 'grey', icon: 'minus' };
}

/** A backup run's result. */
export function backupStatus(ok: boolean): Status {
  return ok ? { word: 'ok', tone: 'green', icon: 'check' } : { word: 'failed', tone: 'red', icon: 'x' };
}
