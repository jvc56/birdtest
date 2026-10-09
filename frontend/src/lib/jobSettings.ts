/**
 * A job's configuration (`GET /api/jobs/:id/config`) and a player config's
 * as their pages show them: tables of the settings that matter most -- the
 * key rows -- with every setting a toggle away. A job's own settings are one
 * ordered list; its players' are another table, side by side.
 */
import type { JobType } from '$lib/api';
import { jobTypeLabel, targetsText } from '$lib/format';

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
    min_magpie_version: string;
  };
  games?: {
    unit: 'game' | 'pair';
    per_batch: number;
    /** Off, the job plays `max_units` and stops; the test's settings are unused. */
    test_enabled: boolean;
    min_units: number;
    max_units: number;
    /** The significance test's confidence, in percent. */
    confidence_pct: number;
    capture_positions: boolean;
    /** Game pairs: only each pair's first divergence is kept. */
    capture_first_divergence: boolean;
  };
  opening_racks?: {
    racks_per_batch: number;
    rack_size: number;
    total_racks: number;
    /** See `lib/consensus.ts`: one analysis per rack when the most is 1. */
    consensus_pct: number;
    min_results_per_rack: number;
    max_results_per_rack: number;
  };
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

/** Whether a player solves its endgames, and to what depth. */
export function endgameText(p: PlayerSettings): string {
  return solves(p) ? `yes (${p.endgame_plies} plies)` : 'no';
}

/** Whether a player solves its pre-endgames: never without the endgame, which it needs. */
export function preEndgameText(p: PlayerSettings): string {
  return solves(p) && p.peg_max_bag > 0 ? `yes (bag ≤ ${p.peg_max_bag})` : 'no';
}

/**
 * How a player searches, in a few words: what tells two configs apart. Its
 * endgame and pre-endgame solving follow the rest of the game's search.
 * Settings named in `unused` (by row id) are left out: an opening-rack job's
 * player never infers, and only a games or pairs job reaches the end of a
 * game.
 */
export function playerSummary(p: PlayerSettings, unused: ReadonlySet<string> = NONE): string {
  let search: string;
  if (p.num_plies === 0) {
    search = `static, by ${p.sort_strategy}`;
  } else {
    const parts = [`${p.num_plies}-ply sim`];
    if (p.max_iterations !== null) parts.push(`${p.max_iterations.toLocaleString()} iterations`);
    if (p.stopping_pct !== null) parts.push(`stops at ${p.stopping_pct}%`);
    if (p.use_inference && !unused.has('use_inference')) parts.push('inference');
    search = parts.join(', ');
  }
  if (unused.has('endgame') || !solves(p)) return search;
  const solving = [`${p.endgame_plies}-ply endgame`];
  if (p.peg_max_bag > 0) solving.push(`PEG ≤${p.peg_max_bag}`);
  return [search, ...solving].join(' · ');
}

/** One of a job's settings. `id` names the setting whatever its label says. */
export interface JobSetting {
  id: string;
  label: string;
  value: string;
}

const setting = (id: string, label: string, value: string): JobSetting => ({ id, label, value });

/**
 * The job's own settings and its type's, as one ordered list, all of them: a
 * job shows the rows that apply to it. First what the job is: its type; the
 * rules every game is played by -- variant, letter distribution, board, bingo
 * bonus; how much it plays -- a games job's target, its significance test and
 * whether it records positions, an opening-rack job's analyses per rack and
 * the agreement that settles a rack, a leave job's generations and each one's
 * target. Then the simulation cutoff, the test's minimum, batch sizes and the
 * oldest MAGPIE. A leave job's lexicon and wordmap are its player's, and shown
 * with it; it has no sim cutoff row, since it never simulates.
 */
export function jobSettings(c: JobConfig): JobSetting[] {
  const g = c.games;
  const o = c.opening_racks;
  const l = c.leave_generation;
  const units = g?.unit === 'pair' ? 'Pairs' : 'Games';
  const rows: JobSetting[] = [
    setting('type', 'Type', jobTypeLabel(c.job.job_type)),
    setting('variant', 'Variant', show(c.job.variant)),
    setting('letter_distribution', 'Letter Distribution', show(c.job.letter_distribution)),
    setting('board', 'Board', show(c.job.layout)),
    setting('bingo_bonus', 'Bingo Bonus', show(c.job.bingo_bonus))
  ];
  if (g) {
    // A job without a test stores the test's defaults, which it never reads:
    // shown, they would read as a test it runs.
    rows.push(
      g.test_enabled
        ? setting('max_units', `Maximum ${units}`, show(g.max_units))
        : setting('max_units', `${units} To Play`, show(g.max_units)),
      setting(
        'test_enabled',
        'Significance Test',
        g.test_enabled ? `yes (${show(g.confidence_pct)}%)` : 'no'
      ),
      setting(
        'capture_positions',
        'Position Recorder',
        g.capture_positions && g.capture_first_divergence ? 'yes (first divergences)' : show(g.capture_positions)
      )
    );
  }
  if (o) {
    rows.push(
      setting('min_results_per_rack', 'Minimum Analyses Per Rack', show(o.min_results_per_rack)),
      setting('max_results_per_rack', 'Maximum Analyses Per Rack', show(o.max_results_per_rack)),
      // One analysis per rack settles each rack at its first: no agreement is sought.
      setting('consensus_pct', 'Consensus %', o.max_results_per_rack > 1 ? `${show(o.consensus_pct)}%` : '—')
    );
  }
  if (l) {
    rows.push(
      setting('generations', 'Generations', show(l.target_rack_counts.length)),
      setting('target_rack_counts', 'Target Per Rack', targetsText(l.target_rack_counts))
    );
  }
  if (!l) rows.push(setting('sim_cutoff', 'Sim Cutoff', show(c.job.sim_cutoff)));
  if (g?.test_enabled) rows.push(setting('min_units', `Minimum ${units}`, show(g.min_units)));
  if (g) rows.push(setting('per_batch', `${units} Per Task`, show(g.per_batch)));
  if (l) rows.push(setting('num_iterations', 'Games Per Task', show(l.num_iterations)));
  if (o) rows.push(setting('racks_per_batch', 'Racks Per Task', show(o.racks_per_batch)));
  if (l) rows.push(setting('racks_per_task', 'Racks Per Task', show(l.racks_per_task)));
  rows.push(setting('min_magpie_version', 'Oldest MAGPIE', show(c.job.min_magpie_version)));
  return rows;
}

/** A player setting as the table lists it. */
interface PlayerRowSpec {
  /** Names the setting whatever its label says. */
  id: string;
  label: string;
  value: (p: PlayerSettings) => string;
  /** Always shown; the rest only under "All settings". */
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
 * it searches, what it keeps and whether it infers and solves the end of the
 * game first (the key rows, always shown), then the rest of each. A row that
 * does not apply to a player shows "—": inference for a static player, the
 * plies one records, and the move-gen margin of a recorder that keeps no
 * moves by equity.
 */
const PLAYER_ROWS: PlayerRowSpec[] = [
  { id: 'lexicon', label: 'Lexicon', value: field('lexicon'), key: true },
  { id: 'leaves', label: 'Leaves', value: field('leaves'), key: true },
  { id: 'sort_strategy', label: 'Sorted By', value: field('sort_strategy'), key: true },
  { id: 'recorder_type', label: 'Move Recorder', value: field('recorder_type'), key: true },
  { id: 'num_plays', label: 'Moves Generated', value: field('num_plays'), key: true },
  { id: 'num_plies', label: 'Plies', value: field('num_plies'), key: true },
  { id: 'use_inference', label: 'Uses Inference', value: field('use_inference'), key: true },
  { id: 'pre_endgame', label: 'Uses Preendgame', value: (p) => preEndgameText(p), key: true },
  { id: 'endgame', label: 'Uses Endgame', value: (p) => endgameText(p), key: true },
  { id: 'win_pct', label: 'Win % Model', value: field('win_pct'), sim: true },
  { id: 'max_iterations', label: 'Maximum Total Iterations', value: field('max_iterations'), sim: true },
  { id: 'stopping_pct', label: 'Stopping %', value: field('stopping_pct'), sim: true },
  { id: 'num_plays_recorded', label: 'Moves Recorded', value: field('num_plays_recorded') },
  {
    id: 'num_plies_recorded',
    label: 'Plies Recorded',
    value: (p) => (p.num_plies === 0 ? '—' : show(p.num_plies_recorded))
  },
  { id: 'min_play_iterations', label: 'Minimum Iterations per Play', value: field('min_play_iterations'), sim: true },
  { id: 'threshold', label: 'Stopping Threshold Rule', value: field('threshold'), sim: true },
  { id: 'sampling_rule', label: 'Sampling Rule', value: field('sampling_rule'), sim: true },
  { id: 'inference_margin', label: 'Inference Margin', value: field('inference_margin'), sim: true },
  { id: 'utility_w_winpct', label: 'Win % Utility Weight', value: field('utility_w_winpct'), sim: true },
  { id: 'utility_w_spread', label: 'Spread Utility Weight', value: field('utility_w_spread'), sim: true },
  { id: 'utility_spread_scale', label: 'Spread Utility Scale', value: field('utility_spread_scale'), sim: true },
  { id: 'time_limit_secs', label: 'Time Limit (Seconds)', value: field('time_limit_secs'), sim: true },
  {
    id: 'movegen_margin',
    label: 'Movegen Margin',
    value: (p) => (p.recorder_type === 'equity' ? show(p.movegen_margin) : '—')
  },
  { id: 'peg_stage_top_k', label: 'PEG Schedule', value: (p) => list(p.peg_stage_top_k) },
  { id: 'peg_scenario_stride', label: 'PEG Stride', value: field('peg_scenario_stride') },
  { id: 'peg_opp_model', label: 'PEG Opponent', value: field('peg_opp_model') },
  { id: 'peg_nested', label: 'Nested Lookahead', value: field('peg_nested') },
  { id: 'peg_nested_cand_caps', label: 'Nested Caps', value: (p) => list(p.peg_nested_cand_caps) },
  { id: 'peg_nested_max_depth', label: 'Nested Depth', value: field('peg_nested_max_depth') },
  { id: 'peg_nested_strides', label: 'Nested Strides', value: (p) => list(p.peg_nested_strides) },
  { id: 'use_wordmap', label: 'Wordmap', value: field('use_wordmap') },
  { id: 'use_rit', label: 'Rack Info Table', value: field('use_rit') },
  { id: 'use_wit', label: 'Word Info Table', value: field('use_wit') }
];

/** A player setting: one value per player, and whether the players differ in it. */
export interface SettingRow {
  id: string;
  label: string;
  values: string[];
  differs: boolean;
  /** The config has it, and the job never reads it; absent when the job does. */
  unused?: true;
}

function row(id: string, label: string, values: string[]): SettingRow {
  return { id, label, values, differs: new Set(values).size > 1 };
}

/** Every endgame and pre-endgame row's id. */
const SOLVER_ROWS = [
  'endgame',
  'pre_endgame',
  'peg_stage_top_k',
  'peg_scenario_stride',
  'peg_opp_model',
  'peg_nested',
  'peg_nested_cand_caps',
  'peg_nested_max_depth',
  'peg_nested_strides'
];

const NONE: ReadonlySet<string> = new Set();

/**
 * What a leave job's bot never reads of its player, by row id: the leaves,
 * because every generation plays the KLV the server built from the one
 * before; the win% model and the moves generated, because it plays
 * statically (autoplay keeps only the best move a static player generates,
 * whatever its list holds); and what is kept of its moves -- the recorder,
 * the plays and plies recorded, and the move-gen margin that bounds an equity
 * recorder -- because it plays the best move and reports only the racks it
 * drew.
 */
const LEAVE_UNUSED: ReadonlySet<string> = new Set([
  'leaves',
  'win_pct',
  'num_plays',
  'recorder_type',
  'num_plays_recorded',
  'num_plies_recorded',
  'movegen_margin',
  // A leave game ends before the bag is small enough for either solver.
  ...SOLVER_ROWS
]);

/**
 * What an opening-rack job never reads of its player: an opening rack has no
 * previous play to infer from (MAGPIE turns inference off for it), and it is
 * analysed on an empty board, far from the endgame.
 */
const OPENING_RACK_UNUSED: ReadonlySet<string> = new Set([
  'use_inference',
  'inference_margin',
  ...SOLVER_ROWS
]);

/**
 * The recorder and the move-gen margin, which only an opening-rack static
 * analysis reads: autoplay generates every move of a games, pairs or leave job
 * with MAGPIE's own record type and a margin of 0, and an opening-rack simmer
 * ranks every play up to its candidate count whatever its recorder says.
 */
const RECORDER_ROWS = ['recorder_type', 'movegen_margin'];

/** What an opening-rack job whose player simulates never reads. */
const OPENING_RACK_SIM_UNUSED: ReadonlySet<string> = new Set([
  ...OPENING_RACK_UNUSED,
  ...RECORDER_ROWS
]);

/** What a games or pairs job that records positions never reads. */
const CAPTURED_UNUSED: ReadonlySet<string> = new Set(RECORDER_ROWS);

/**
 * What a games or pairs job that records no positions never reads: the plays
 * and plies recorded say what a captured position keeps, and nothing else.
 */
const UNCAPTURED_UNUSED: ReadonlySet<string> = new Set([
  ...RECORDER_ROWS,
  'num_plays_recorded',
  'num_plies_recorded'
]);

/**
 * What a games or pairs job that records no positions never reads when no
 * player simulates: a static player's moves generated only size the list
 * autoplay keeps its best move in, and only a captured position (or a
 * simmer's candidates) reads more of it.
 */
const UNCAPTURED_STATIC_UNUSED: ReadonlySet<string> = new Set([...UNCAPTURED_UNUSED, 'num_plays']);

/** The player settings, by row id, that a job never reads. */
export function unusedPlayerSettings(c: JobConfig): ReadonlySet<string> {
  const simulates = c.players.some((p) => p.num_plies > 0);
  if (c.job.job_type === 'leave_generation') return LEAVE_UNUSED;
  if (c.job.job_type === 'opening_rack') {
    return simulates ? OPENING_RACK_SIM_UNUSED : OPENING_RACK_UNUSED;
  }
  if (c.games && !c.games.capture_positions) {
    // The rows are the job's, not each player's: a simmer's candidate count
    // is read, so the row stays live beside one.
    return simulates ? UNCAPTURED_UNUSED : UNCAPTURED_STATIC_UNUSED;
  }
  return CAPTURED_UNUSED;
}

function marked(rows: SettingRow[], unused: ReadonlySet<string>): SettingRow[] {
  // A setting the job never reads cannot tell its players apart, so it is
  // never marked as a difference.
  return rows.map((r) => (unused.has(r.id) ? { ...r, unused: true, differs: false } : r));
}

/**
 * The rows that apply to these players: every key row, and of the rest the
 * simulation rows only when one of them simulates (a static player shows "—"
 * in them beside a simmer).
 */
function rowsFor(players: PlayerSettings[], keyOnly: boolean, unused: ReadonlySet<string>): SettingRow[] {
  const simulates = players.some((p) => p.num_plies > 0);
  return marked(
    PLAYER_ROWS.filter((spec) => !keyOnly || spec.key)
      .filter((spec) => simulates || !spec.sim)
      .map((spec) => row(spec.id, spec.label, players.map(spec.value))),
    unused
  );
}

/**
 * What a reader compares configs by first -- the files, the search, what is
 * kept, whether it infers and how the end of the game is solved -- one value
 * per player. `playerRows` has every setting. Rows whose ids are in `unused`
 * are marked so.
 */
export function keySettings(players: PlayerSettings[], unused = NONE): SettingRow[] {
  return rowsFor(players, true, unused);
}

/**
 * Every setting, one value per player, in the key rows' order with the rest
 * after them. Rows whose ids are in `unused` are marked so.
 */
export function playerRows(players: PlayerSettings[], unused = NONE): SettingRow[] {
  return rowsFor(players, false, unused);
}

/** A player settings table's rows, "differences first". */
export interface SettingBlocks {
  /** Every setting the players differ in, key or not; empty for one player. */
  differences: SettingRow[];
  /** The rest of the rows asked for: the key rows, or with `all` every one. */
  shared: SettingRow[];
}

/**
 * The rows of a player settings table, split for reading differences first:
 * what the players differ in, from every setting -- a difference is what a
 * reader of two configs is looking for, and a non-key one hidden behind
 * "All settings" was missed -- then what they share, the key rows unless
 * `all`. One player shares everything with itself.
 */
export function settingBlocks(players: PlayerSettings[], all: boolean, unused = NONE): SettingBlocks {
  const listed = all ? playerRows(players, unused) : keySettings(players, unused);
  if (players.length < 2) return { differences: [], shared: listed };
  return {
    differences: playerRows(players, unused).filter((r) => r.differs),
    shared: listed.filter((r) => !r.differs)
  };
}

/** The players' searches in one line, for beside a job's lexicon and variant. */
export function playersLine(c: JobConfig): string {
  const unused = unusedPlayerSettings(c);
  return c.players.map((p) => playerSummary(p, unused)).join(' vs ');
}
