import type { Contribution, ContributionsByType, Contributor, JobType } from '$lib/api';
import { jobTypeLabel } from '$lib/format';

/**
 * The job types in the order the Contributions page lists their work: the
 * order a job is created in, the one the job list's type names follow.
 */
export const MOVEGEN_TYPES: JobType[] = ['opening_rack', 'games', 'game_pairs', 'leave_generation'];

/** One row of a breakdown: the type, its label and its work. */
export interface ContributionLine extends Contribution {
  type: JobType;
  label: string;
}

const NOTHING: Contribution = { movegens: 0, compute_seconds: 0, tasks: 0 };

/**
 * A breakdown's rows, every type in `MOVEGEN_TYPES` order -- one the server
 * left out reads 0, so the four are always listed and always in the same
 * places, whichever a contributor worked on.
 */
export function contributionLines(byType: Partial<ContributionsByType>): ContributionLine[] {
  return MOVEGEN_TYPES.map((type) => ({ type, label: jobTypeLabel(type), ...(byType[type] ?? NOTHING) }));
}

/** The rows' totals: the site's, across every job type. */
export function contributionTotals(lines: Contribution[]): Contribution {
  return lines.reduce(
    (sum, line) => ({
      movegens: sum.movegens + line.movegens,
      compute_seconds: sum.compute_seconds + line.compute_seconds,
      tasks: sum.tasks + line.tasks
    }),
    NOTHING
  );
}

/**
 * Who a contributor list row is, as the breakdown's route names them -- an
 * account by its id, an anonymous worker by its pseudonym, never by the UUID
 * that is its credential -- or null for a row with neither, which has no
 * breakdown to ask for. Also the key an open breakdown is kept under, so it
 * stays open while the list refreshes around it.
 */
export function contributorKey(worker: Pick<Contributor, 'user_id' | 'anon_id'>): string | null {
  if (worker.user_id) return `user/${encodeURIComponent(worker.user_id)}`;
  if (worker.anon_id) return `anon/${encodeURIComponent(worker.anon_id)}`;
  return null;
}
