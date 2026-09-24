/**
 * The arithmetic behind RatingHistoryChart.svelte: which configs are drawn,
 * in which colour, and where.
 */
import type { RatingHistoryPoint } from '$lib/api';

/**
 * At most this many configs are drawn. A categorical palette is a fixed list,
 * not a generator: past its length the honest move is to show fewer series
 * and say so, rather than invent hues nobody can tell apart.
 */
export const SERIES_CAP = 6;

// Validated for this surface (dark, #151922) at all-pairs CVD separation.
// See scripts/validate_palette.js in the dataviz reference.
export const PALETTE = ['#027FD1', '#BF2001', '#7902F0', '#03856D', '#E103AE', '#A09100'];

export const PAD = { top: 12, right: 132, bottom: 28, left: 52 };
export const HEIGHT = 260;

export interface Series {
  id: string;
  name: string;
  color: string;
  points: { t: number; rating: number }[];
}

export interface Domain {
  t0: number;
  t1: number;
  r0: number;
  r1: number;
}

/** FNV-1a over the id: a stable, well-spread number per config. */
function hashId(id: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    hash ^= id.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/** The palette slot a config asks for, before any collision is resolved. */
export function preferredSlot(id: string): number {
  return hashId(id) % PALETTE.length;
}

/**
 * Colour by config identity, not by rank, so a config keeps its colour from
 * one fit to the next while ratings cross, and when the set drawn changes.
 *
 * Each config asks for the slot its id hashes to and gets it unless a drawn
 * config with a smaller id asks for the same one. Removing configs from the
 * set can only remove competitors, so a config holding its own slot keeps it
 * under any filtering. The losers of a clash take the free slots left over
 * (in id order, searching onward from the slot they asked for), so the
 * colours on screen are always distinct: at most SERIES_CAP ids are drawn,
 * and there are PALETTE.length slots.
 */
export function assignColors(ids: string[]): Map<string, string> {
  const sorted = [...new Set(ids)].sort();
  const slotOf = new Map<string, number>();
  const taken = new Set<number>();
  const displaced: string[] = [];
  for (const id of sorted) {
    const slot = preferredSlot(id);
    if (taken.has(slot)) {
      displaced.push(id);
    } else {
      taken.add(slot);
      slotOf.set(id, slot);
    }
  }
  for (const id of displaced) {
    let slot = preferredSlot(id);
    for (let probe = 0; probe < PALETTE.length && taken.has(slot); probe++) {
      slot = (slot + 1) % PALETTE.length;
    }
    taken.add(slot);
    slotOf.set(id, slot);
  }
  return new Map(sorted.map((id) => [id, PALETTE[slotOf.get(id)!]]));
}

/**
 * One series per config, oldest point first. Ranked by latest rating so the
 * cap keeps the configs a reader is looking for; `hidden` counts the rest.
 */
export function groupHistory(history: RatingHistoryPoint[]): {
  series: Series[];
  hidden: number;
} {
  const byConfig = new Map<string, RatingHistoryPoint[]>();
  for (const point of history) {
    const list = byConfig.get(point.player_config_id) ?? [];
    list.push(point);
    byConfig.set(point.player_config_id, list);
  }
  const grouped = [...byConfig.entries()].map(([id, points]) => ({
    id,
    name: points[0].name,
    // The API returns history oldest first; sorting again costs nothing and
    // makes "latest" mean latest whatever order the points arrive in.
    points: points
      .map((p) => ({ t: Date.parse(p.computed_at), rating: p.rating }))
      .sort((a, b) => a.t - b.t)
  }));
  const ranked = grouped.sort(
    (a, b) => (b.points.at(-1)?.rating ?? 0) - (a.points.at(-1)?.rating ?? 0)
  );
  const shown = ranked.slice(0, SERIES_CAP);
  const colors = assignColors(shown.map((s) => s.id));
  return {
    series: shown.map((s) => ({ ...s, color: colors.get(s.id)! })),
    hidden: Math.max(0, ranked.length - SERIES_CAP)
  };
}

/** The drawn extent, or null while there is too little history to draw. */
export function historyDomain(series: Series[]): Domain | null {
  const all = series.flatMap((s) => s.points);
  if (all.length < 2) return null;
  const ts = all.map((p) => p.t);
  const rs = all.map((p) => p.rating);
  const lo = Math.min(...rs);
  const hi = Math.max(...rs);
  const pad = Math.max(5, (hi - lo) * 0.1);
  return { t0: Math.min(...ts), t1: Math.max(...ts), r0: lo - pad, r1: hi + pad };
}

export function historyX(t: number, d: Domain, width: number): number {
  return (
    PAD.left + (d.t1 === d.t0 ? 0 : ((t - d.t0) / (d.t1 - d.t0)) * (width - PAD.left - PAD.right))
  );
}

export function historyY(r: number, d: Domain): number {
  return PAD.top + (1 - (r - d.r0) / (d.r1 - d.r0)) * (HEIGHT - PAD.top - PAD.bottom);
}

/** The SVG path for one series. */
export function seriesPath(series: Series, d: Domain, width: number): string {
  return series.points
    .map((p, i) => `${i === 0 ? 'M' : 'L'}${historyX(p.t, d, width)},${historyY(p.rating, d)}`)
    .join(' ');
}
