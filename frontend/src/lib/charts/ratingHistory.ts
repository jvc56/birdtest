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

/**
 * The most characters an end label shows. The right margin is fixed, and an
 * SVG clips what runs past it: a name that did lost its tail -- usually the
 * part that tells two configs apart (`…-gk16`, `…-none`) -- and two such
 * labels read the same (the audit's pass 25). 16 characters at 11px fit the
 * margin in lower case and ordinary mixed case; a name in wide capitals can
 * still run past it (KL-76), and the label's title holds the whole name.
 */
export const LABEL_CHARS = 16;

/**
 * `name`, shortened in the middle to at most `max` characters: its start
 * and, longer, its end, which is where configs named in a family differ.
 * The full name goes in the label's title.
 */
export function fitLabel(name: string, max: number = LABEL_CHARS): string {
  if (name.length <= max) return name;
  const head = Math.max(1, Math.floor((max - 1) * 0.4));
  return `${name.slice(0, head)}…${name.slice(name.length - (max - 1 - head))}`;
}

/**
 * Labels for `names` drawn together, each at most `max` characters, told
 * apart from one another: shortened one at a time, `simmer-CSW24-2ply-…-off`
 * and `simmer-CSW24-4ply-…-off` read the same, since they differ in the
 * middle. Segments every config of a family (the part before the first `-`)
 * shares are dropped first -- the table below names them in full -- and two
 * labels still alike are widened around the first character where they differ.
 */
export function fitLabels(names: string[], max: number = LABEL_CHARS): string[] {
  const family = (name: string) => name.split('-')[0];
  const trimmed = names.map((name) => {
    if (name.length <= max) return name;
    const kin = names.filter((other) => other !== name && family(other) === family(name));
    if (!kin.length) return name;
    const segments = name.split('-');
    let shared = 0;
    while (
      shared < segments.length - 1 &&
      kin.every((other) => other.split('-')[shared] === segments[shared])
    ) {
      shared++;
    }
    return shared ? segments.slice(shared).join('-') : name;
  });
  const labels = trimmed.map((t) => fitLabel(t, max));
  return labels.map((label, i) => {
    const twins = labels.flatMap((other, j) => (j !== i && other === label ? [j] : []));
    if (!twins.length) return label;
    const own = trimmed[i];
    let differ = 0;
    while (differ < own.length && twins.every((j) => trimmed[j][differ] === own[differ])) differ++;
    const start = Math.max(0, Math.min(differ - 4, own.length - (max - 1)));
    return start > 0 ? `…${own.slice(start, start + max - 1)}` : own.slice(0, max);
  });
}

/**
 * Label positions for points at `ys`, in the same order, moved apart until
 * each is at least `gap` from the next and all lie within `lo`..`hi`: two
 * configs a few points apart were labelled on top of each other.
 */
export function spreadLabels(ys: number[], gap: number, lo: number, hi: number): number[] {
  const order = ys.map((y, i) => ({ y, i })).sort((a, b) => a.y - b.y || a.i - b.i);
  const placed = order.map((o) => o.y);
  for (let k = 0; k < placed.length; k++) {
    placed[k] = Math.max(placed[k], k === 0 ? lo : placed[k - 1] + gap);
  }
  // Pushed past the bottom: move the run back up, keeping the gaps.
  for (let k = placed.length - 1; k >= 0; k--) {
    placed[k] = Math.min(placed[k], k === placed.length - 1 ? hi : placed[k + 1] - gap);
  }
  const out = new Array<number>(ys.length);
  order.forEach((o, k) => (out[o.i] = placed[k]));
  return out;
}
