/**
 * Who is in a rating pool, as the pool page's membership controls need it.
 *
 * Built from the detail's `members` -- the membership table -- and not from
 * `ratings`, the latest fit's rows: the two differ whenever a member was added
 * or removed since that fit (or its refit failed), and the page built from the
 * ratings offered an unrated member under "Add" and gave it no Remove button.
 */
import type {
  PlayerConfig,
  RatingHeadToHead,
  RatingPoolDetail,
  RatingPoolMember,
  RatingRow
} from '$lib/api';
import { formatResidual, residual } from '$lib/charts/residuals';
import { scorePct } from '$lib/format';

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

/**
 * The cross table: every rated config against every other, from the row's
 * side, the rating in the last column.
 *
 * Its configs are the latest fit's, best first, and then the configs no chain
 * of games links to the anchor -- whose ratings are the prior's and mean
 * nothing -- so the rating column reads down in order.
 */
export interface CrossTable {
  configs: RatingRow[];
  /** The row config's cell against the column's, if they have played. */
  cell: (row: string, col: string) => RatingHeadToHead | undefined;
}

export function crossTable(pool: Pick<RatingPoolDetail, 'ratings' | 'head_to_heads'>): CrossTable {
  const configs = [...pool.ratings].sort(
    (a, b) => Number(b.connected_to_anchor) - Number(a.connected_to_anchor) || b.rating - a.rating
  );
  const cells = new Map(pool.head_to_heads.map((c) => [`${c.row}:${c.col}`, c]));
  return { configs, cell: (row, col) => cells.get(`${row}:${col}`) };
}

/**
 * Each head-to-head once, from the side of the config whose id sorts first:
 * what the residual checks count, where a cell and its mirror would be the
 * same miss twice.
 */
export function oneSide(cells: RatingHeadToHead[]): RatingHeadToHead[] {
  return cells.filter((c) => c.row < c.col);
}

/** Signed to one decimal, with no "-0.0" for a figure that rounds to zero. */
function signed(n: number): string {
  const rounded = Number(n.toFixed(1));
  return `${rounded > 0 ? '+' : ''}${(rounded === 0 ? 0 : rounded).toFixed(1)}`;
}

/** The win % ± its standard error, in percentage points: "58.8% ±6.2". */
export function winText(cell: RatingHeadToHead): string {
  const error = Number.isFinite(cell.stderr) ? (100 * cell.stderr).toFixed(1) : '∞';
  return `${scorePct(cell.actual)} ±${error}`;
}

/** The average spread per game, signed: "+6.8", "-12.5", "0.0". */
export function spreadText(cell: RatingHeadToHead): string {
  return signed(cell.spread);
}

/**
 * The cell's hover: what it shows, spelled out, and what the ratings predict
 * -- the residual, which a single rating per config cannot remove where the
 * pool is non-transitive.
 */
export function cellTitle(cell: RatingHeadToHead, rowName: string, colName: string): string {
  const pairs = `${cell.pairs.toLocaleString('en-US')} pair${cell.pairs === 1 ? '' : 's'}`;
  return (
    `${rowName} against ${colName}: ${winText(cell)} over ${pairs}, ` +
    `average spread ${spreadText(cell)}. The ratings predict ${scorePct(cell.predicted)} ` +
    `(residual ${formatResidual(residual(cell))} percentage points).`
  );
}
