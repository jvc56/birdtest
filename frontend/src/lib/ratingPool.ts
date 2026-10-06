/**
 * Who is in a rating pool, as the pool page's membership controls need it.
 *
 * Built from the detail's `members` -- the membership table -- and not from
 * `ratings`, the latest fit's rows: the two differ whenever a member was added
 * or removed since that fit (or its refit failed), and the page built from the
 * ratings offered an unrated member under "Add" and gave it no Remove button.
 */
import type { PlayerConfig, RatingPoolDetail, RatingPoolMember } from '$lib/api';

export interface PoolMembership {
  /** Config ids in the pool now. */
  memberIds: Set<string>;
  /** Members the latest fit has not rated, in the detail's (name) order. */
  unrated: RatingPoolMember[];
  /** Configs that are not members: what "Add" offers. */
  others: PlayerConfig[];
}

export function poolMembership(pool: RatingPoolDetail, configs: PlayerConfig[]): PoolMembership {
  const memberIds = new Set(pool.members.map((m) => m.player_config_id));
  const rated = new Set(pool.ratings.map((r) => r.player_config_id));
  return {
    memberIds,
    unrated: pool.members.filter((m) => !rated.has(m.player_config_id)),
    others: configs.filter((c) => !memberIds.has(c.id))
  };
}
