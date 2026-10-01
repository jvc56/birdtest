/**
 * A job's configuration (`GET /api/jobs/:id/config`) and a player config's
 * as their pages show them: tables of the settings that matter most -- the
 * key rows -- with every setting a toggle away. A job's own settings are one
 * ordered list; its players' are another table, side by side.
 */
import type { JobType } from '$lib/api';
import { jobTypeLabel } from '$lib/format';

export interface PlayerSettings {
  /** Its part in a job ("player 1"); absent for a config read on its own. */
  role?: string;
  id: string;
  name: string;
  lexicon: string;
  leaves: string;
  win_pct: string | null;
  recorder_type: string;
  sort_strategy: string;
  num_plies: number;
  num_plies_recorded: number;
  num_plays: number;
  num_plays_recorded: number;
  max_iterations: number | null;
  stopping_pct: number | null;
  use_inference: boolean | null;
  time_limit_secs: number | null;
  use_wordmap: boolean;
  use_rit: boolean;
  use_wit: boolean;
  min_play_iterations: number | null;
  threshold: string | null;
  sampling_rule: string | null;
  inference_margin: number | null;
  utility_w_winpct: number | null;
  utility_w_spread: number | null;
  utility_spread_scale: number | null;
  movegen_margin: number;
  /** 0 solves nothing; the pre-endgame needs it above 0 too. */
  endgame_plies: number;
  peg_max_bag: number;
  /** Null unless the pre-endgame runs. */
  peg_stage_top_k: number[] | null;
  peg_scenario_stride: number | null;
  peg_opp_model: string | null;
  peg_nested: boolean | null;
  /** Null unless nested lookahead is on. */
  peg_nested_cand_caps: number[] | null;
  peg_nested_max_depth: number | null;
  peg_nested_strides: number[] | null;
}

/** A player config as anyone may read it (`GET /api/player-configs`). */
export interface PublicPlayerConfig extends PlayerSettings {
  cloned_from_id: string | null;
  created_at: string;
}

export interface JobConfig {
  job: {
    id: string;
    name: string;
    job_type: JobType;
    variant: string;
    letter_distribution: string;
    layout: string;
    bingo_bonus: number;
    sim_cutoff: number;
    redundancy: number;
    min_magpie_version: string;
  };
  games?: {
    unit: 'game' | 'pair';
    per_batch: number;
    /** Off, the job plays `max_units` and stops; the test's settings are unused. */
    sprt_enabled: boolean;
    min_units: number;
    max_units: number;
    sprt_alpha: number;
    sprt_beta: number;
    elo_low: number;
    elo_high: number;
    capture_positions: boolean;
    /** Game pairs: only each pair's first divergence is kept. */
    capture_first_divergence: boolean;
  };
  opening_racks?: { racks_per_batch: number; rack_size: number; total_racks: number };
  /** Its lexicon and wordmap setting are its player's, in `players`. */
  leave_generation?: {
    num_iterations: number;
    /** One occurrence target per generation, in order; its length is the generation count. */
    target_rack_counts: number[];
    racks_per_task: number;
  };
  players: PlayerSettings[];
}

type Value = string | number | boolean | null | undefined;

/** A setting as shown: blank is "—", a flag yes or no, a number grouped. */
export function show(value: Value): string {
  if (value === null || value === undefined || value === '') return '—';
  if (typeof value === 'boolean') return value ? 'yes' : 'no';
  if (typeof value === 'number') return value.toLocaleString();
  return value;
}

/** Whether a player solves its endgames, and so perhaps its pre-endgames. */
const solves = (p: PlayerSettings) => p.endgame_plies > 0;

/** The endgame a player solves, in a few words. */
export function endgameText(p: PlayerSettings): string {
  return solves(p) ? `${p.endgame_plies}-ply endgame` : 'off';
}

/** The pre-endgame a player solves: off without the endgame, which it needs. */
export function preEndgameText(p: PlayerSettings): string {
  return solves(p) && p.peg_max_bag > 0 ? `bag ≤ ${p.peg_max_bag}` : 'off';
}

/**
 * How a player searches, in a few words: what tells two configs apart. Its
 * endgame and pre-endgame solving follow the rest of the game's search.
 * Settings named in `unused` (by row label) are left out: an opening-rack
 * job's player never infers, and only a games or pairs job reaches the end of
 * a game.
 */
export function playerSummary(p: PlayerSettings, unused: ReadonlySet<string> = NONE): string {
  let search: string;
  if (p.num_plies === 0) {
    search = `static, by ${p.sort_strategy}`;
  } else {
    const parts = [`${p.num_plies}-ply sim`];
    if (p.max_iterations !== null) parts.push(`${p.max_iterations.toLocaleString()} iterations`);
    if (p.stopping_pct !== null) parts.push(`stops at ${p.stopping_pct}%`);
    if (p.use_inference && !unused.has('Inference')) parts.push('inference');
    search = parts.join(', ');
  }
  if (unused.has('Endgame') || !solves(p)) return search;
  const solving = [endgameText(p)];
  if (p.peg_max_bag > 0) solving.push(`PEG ≤${p.peg_max_bag}`);
  return [search, ...solving].join(' · ');
}

/** One of a job's settings; `key` if it is shown before "All settings". */
export interface JobSetting {
  label: string;
  value: string;
  key: boolean;
}


/** A key setting and one shown only under "All settings". */
const key = (label: string, value: string): JobSetting => ({ label, value, key: true });
const more = (label: string, value: string): JobSetting => ({ label, value, key: false });

/**
 * The job's own settings and its type's, as one ordered list: a job shows the
 * rows that apply to it. The key rows say what the job is: its type; the
 * rules every game is played by -- variant, letter distribution, board, bingo
 * bonus; how much it plays -- a games job's target, its test and whether it
 * records positions, an opening-rack job's racks and their size, a leave job's
 * generations and each one's target. The rest -- the simulation cutoff, the
 * test's minimum and error rates, batch sizes, redundancy, the oldest MAGPIE --
 * is under "All settings". A leave job's lexicon and wordmap are its player's,
 * and shown with it; it has no sim cutoff row, since it never simulates.
 */
export function jobSettings(c: JobConfig): JobSetting[] {
  const g = c.games;
  const o = c.opening_racks;
  const l = c.leave_generation;
  const unit = g?.unit === 'pair' ? 'pair' : 'game';
  const units = g?.unit === 'pair' ? 'Pairs' : 'Games';
  const rows: JobSetting[] = [
    key('Type', jobTypeLabel(c.job.job_type)),
    key('Variant', show(c.job.variant)),
    key('Letter distribution', show(c.job.letter_distribution)),
    key('Board', show(c.job.layout)),
    key('Bingo bonus', show(c.job.bingo_bonus))
  ];
  if (g) {
    // A job without a test stores the test's defaults, which it never reads:
    // shown, they would read as a test it runs.
    rows.push(
      g.sprt_enabled
        ? key(`Cap (${unit}s)`, show(g.max_units))
        : key(`${units} to play`, show(g.max_units)),
      key('SPRT', g.sprt_enabled ? `Elo ${show(g.elo_low)} → ${show(g.elo_high)}` : 'none'),
      key(
        'Records positions',
        g.capture_positions && g.capture_first_divergence ? 'first divergences' : show(g.capture_positions)
      )
    );
  }
  if (o) rows.push(key('Racks in all', show(o.total_racks)), key('Rack size', show(o.rack_size)));
  if (l) {
    rows.push(
      key('Generations', show(l.target_rack_counts.length)),
      key('Target per rack', l.target_rack_counts.map((t) => t.toLocaleString()).join(', '))
    );
  }
  if (!l) rows.push(more('Sim cutoff', show(c.job.sim_cutoff)));
  if (g?.sprt_enabled) {
    rows.push(
      more(`Fewest ${unit}s before the test is acted on`, show(g.min_units)),
      more('SPRT α', show(g.sprt_alpha)),
      more('SPRT β', show(g.sprt_beta))
    );
  }
  if (g) rows.push(more(`${units} per task`, show(g.per_batch)));
  if (l) rows.push(more('Games per task', show(l.num_iterations)));
  if (o) rows.push(more('Racks per task', show(o.racks_per_batch)));
  if (l) rows.push(more('Racks per task', show(l.racks_per_task)));
  rows.push(
    more('Redundancy', `${c.job.redundancy}×`),
    more('Oldest MAGPIE', show(c.job.min_magpie_version))
  );
  return rows;
}

/** A player setting as the table lists it. */
interface PlayerRowSpec {
  label: string;
  value: (p: PlayerSettings) => string;
  /** Shown before "All settings". */
  key?: true;
  /** A simulation setting: "—" for a static player, hidden when all are. */
  sim?: true;
}

const list = (values: number[] | null) => (values ? values.join(', ') : '—');
const field =
  (k: keyof PlayerSettings) =>
  (p: PlayerSettings): string =>
    show(p[k] as Value);

/**
 * Every player setting, in the order a reader compares them: the files, how
 * it searches and what it keeps, and how it solves the end of the game first
 * (the key rows), then the rest of each.
 */
const PLAYER_ROWS: PlayerRowSpec[] = [
  { label: 'Lexicon', value: field('lexicon'), key: true },
  { label: 'Leaves', value: field('leaves'), key: true },
  { label: 'Plies', value: field('num_plies'), key: true },
  { label: 'Plays considered', value: field('num_plays'), key: true },
  { label: 'Sort', value: field('sort_strategy'), key: true },
  { label: 'Win % model', value: field('win_pct'), key: true, sim: true },
  { label: 'Iterations (most)', value: field('max_iterations'), key: true, sim: true },
  { label: 'Stopping %', value: field('stopping_pct'), key: true, sim: true },
  { label: 'Inference', value: field('use_inference'), key: true, sim: true },
  { label: 'Recorder', value: field('recorder_type'), key: true },
  { label: 'Plays recorded', value: field('num_plays_recorded'), key: true },
  { label: 'Endgame', value: (p) => endgameText(p), key: true },
  { label: 'Pre-endgame', value: (p) => preEndgameText(p), key: true },
  { label: 'Plies recorded', value: field('num_plies_recorded') },
  { label: 'Iterations per play (fewest)', value: field('min_play_iterations'), sim: true },
  { label: 'Threshold', value: field('threshold'), sim: true },
  { label: 'Sampling rule', value: field('sampling_rule'), sim: true },
  { label: 'Inference margin', value: field('inference_margin'), sim: true },
  { label: 'Utility weight: win %', value: field('utility_w_winpct'), sim: true },
  { label: 'Utility weight: spread', value: field('utility_w_spread'), sim: true },
  { label: 'Utility spread scale', value: field('utility_spread_scale'), sim: true },
  { label: 'Time limit (s)', value: field('time_limit_secs'), sim: true },
  { label: 'Move-gen margin', value: field('movegen_margin') },
  { label: 'PEG schedule', value: (p) => list(p.peg_stage_top_k) },
  { label: 'PEG stride', value: field('peg_scenario_stride') },
  { label: 'PEG opponent', value: field('peg_opp_model') },
  { label: 'Nested lookahead', value: field('peg_nested') },
  { label: 'Nested caps', value: (p) => list(p.peg_nested_cand_caps) },
  { label: 'Nested depth', value: field('peg_nested_max_depth') },
  { label: 'Nested strides', value: (p) => list(p.peg_nested_strides) },
  { label: 'Wordmap', value: field('use_wordmap') },
  { label: 'Rack info table', value: field('use_rit') },
  { label: 'Word info table', value: field('use_wit') }
];

/** A player setting: one value per player, and whether the players differ in it. */
export interface SettingRow {
  label: string;
  values: string[];
  differs: boolean;
  /** The config has it, and the job never reads it; absent when the job does. */
  unused?: true;
}

function row(label: string, values: string[]): SettingRow {
  return { label, values, differs: new Set(values).size > 1 };
}

/** Every endgame and pre-endgame row's label. */
const SOLVER_ROWS = [
  'Endgame',
  'Pre-endgame',
  'PEG schedule',
  'PEG stride',
  'PEG opponent',
  'Nested lookahead',
  'Nested caps',
  'Nested depth',
  'Nested strides'
];

const NONE: ReadonlySet<string> = new Set();

/**
 * What a leave job's bot never reads of its player: the leaves, because every
 * generation plays the KLV the server built from the one before; the win%
 * model, because it plays statically; and what is kept of its moves -- the
 * recorder, the plays and plies recorded, and the move-gen margin that bounds
 * an equity recorder -- because it plays the best move and reports only the
 * racks it drew.
 */
const LEAVE_UNUSED: ReadonlySet<string> = new Set([
  'Leaves',
  'Win % model',
  'Recorder',
  'Plays recorded',
  'Plies recorded',
  'Move-gen margin',
  // A leave game ends before the bag is small enough for either solver.
  ...SOLVER_ROWS
]);

/**
 * What an opening-rack job never reads of its player: an opening rack has no
 * previous play to infer from (MAGPIE turns inference off for it), and it is
 * analysed on an empty board, far from the endgame.
 */
const OPENING_RACK_UNUSED: ReadonlySet<string> = new Set([
  'Inference',
  'Inference margin',
  ...SOLVER_ROWS
]);

/**
 * What a games or pairs job that records no positions never reads: the plays
 * and plies recorded say what a captured position keeps, and nothing else.
 */
const UNCAPTURED_UNUSED: ReadonlySet<string> = new Set(['Plays recorded', 'Plies recorded']);

/** The player settings, by row label, that a job never reads. */
export function unusedPlayerSettings(c: JobConfig): ReadonlySet<string> {
  if (c.job.job_type === 'leave_generation') return LEAVE_UNUSED;
  if (c.job.job_type === 'opening_rack') return OPENING_RACK_UNUSED;
  if (c.games && !c.games.capture_positions) return UNCAPTURED_UNUSED;
  return NONE;
}

function marked(rows: SettingRow[], unused: ReadonlySet<string>): SettingRow[] {
  return rows.map((r) => (unused.has(r.label) ? { ...r, unused: true } : r));
}

/**
 * The rows that apply to these players: the simulation rows only when one of
 * them simulates (a static player shows "—" in them beside a simmer), and, in
 * the key rows, the endgame's only when one of them solves.
 */
function rowsFor(players: PlayerSettings[], keyOnly: boolean, unused: ReadonlySet<string>): SettingRow[] {
  const simulates = players.some((p) => p.num_plies > 0);
  const solving = players.some(solves);
  return marked(
    PLAYER_ROWS.filter((spec) => !keyOnly || spec.key)
      .filter((spec) => simulates || !spec.sim)
      .filter((spec) => !keyOnly || solving || !SOLVER_ROWS.includes(spec.label))
      .map((spec) => row(spec.label, players.map(spec.value))),
    unused
  );
}

/**
 * What a reader compares configs by first -- the files, the search, what is
 * kept and how the end of the game is solved -- one value per player.
 * `playerRows` has every setting. Rows named in `unused` are marked so.
 */
export function keySettings(players: PlayerSettings[], unused = NONE): SettingRow[] {
  return rowsFor(players, true, unused);
}

/**
 * Every setting, one value per player, in the key rows' order with the rest
 * after them. Rows named in `unused` are marked so.
 */
export function playerRows(players: PlayerSettings[], unused = NONE): SettingRow[] {
  return rowsFor(players, false, unused);
}

/** The players' searches in one line, for beside a job's lexicon and variant. */
export function playersLine(c: JobConfig): string {
  const unused = unusedPlayerSettings(c);
  return c.players.map((p) => playerSummary(p, unused)).join(' vs ');
}
