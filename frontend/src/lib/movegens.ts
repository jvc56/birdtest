import type { Contributor, JobType, MovegensByType } from '$lib/api';
import { jobTypeLabel } from '$lib/format';

/**
 * The job types in the order the Contributors page lists their movegens: the
 * order a job is created in, the one the job list's type names follow.
 */
export const MOVEGEN_TYPES: JobType[] = ['opening_rack', 'games', 'game_pairs', 'leave_generation'];

/** One line of a breakdown: the type, its label and its movegens. */
export interface MovegensLine {
  type: JobType;
  label: string;
  movegens: number;
}

/**
 * A breakdown's lines, every type in `MOVEGEN_TYPES` order -- one the server
 * left out reads 0, so the four are always listed and always in the same
 * places, whichever a contributor worked on.
 */
export function movegensLines(byType: Partial<MovegensByType>): MovegensLine[] {
  return MOVEGEN_TYPES.map((type) => ({
    type,
    label: jobTypeLabel(type),
    movegens: byType[type] ?? 0
  }));
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
