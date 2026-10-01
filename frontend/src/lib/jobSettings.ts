/**
 * A job's configuration (`GET /api/jobs/:id/config`) and a player config's
 * as their pages show them: tables of the settings that matter most -- the
 * key rows -- with every setting a toggle away. A job's own settings are one
 * table, grouped; its players' are another, side by side.
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
 * `unusedSolvers` leaves them out where the job never reaches them.
 */
export function playerSummary(p: PlayerSettings, unusedSolvers = false): string {
  let search: string;
  if (p.num_plies === 0) {
    search = `static, by ${p.sort_strategy}`;
  } else {
    const parts = [`${p.num_plies}-ply sim`];
    if (p.max_iterations !== null) parts.push(`${p.max_iterations.toLocaleString()} iterations`);
    if (p.stopping_pct !== null) parts.push(`stops at ${p.stopping_pct}%`);
    if (p.use_inference) parts.push('inference');
    search = parts.join(', ');
  }
  if (unusedSolvers || !solves(p)) return search;
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

export interface SettingGroup {
  title: string;
  rows: JobSetting[];
}

/** A key setting and one shown only under "All settings". */
const key = (label: string, value: string): JobSetting => ({ label, value, key: true });
const more = (label: string, value: string): JobSetting => ({ label, value, key: false });

/**
 * The job's own settings and its type's, grouped and labelled. The key rows
 * say what the job is: its type; the variant, letter distribution and board
 * every game is played on; how much it plays -- a games job's target, and
 * whether a test can stop it sooner and between which Elo bounds; an
 * opening-rack job's racks and their size; a leave job's generations, each
 * one's target and the games a task plays. The rest -- batch sizes, bingo
 * bonus, sim cutoff, redundancy, the oldest MAGPIE, the test's error rates and
 * minimum, whether positions are recorded -- is under "All settings". A leave
 * job's lexicon and wordmap are its player's, and shown with it.
 */
export function jobGroups(c: JobConfig): SettingGroup[] {
  const groups: SettingGroup[] = [
    {
      title: 'Job',
      rows: [
        key('Type', jobTypeLabel(c.job.job_type)),
        key('Variant', show(c.job.variant)),
        key('Letter distribution', show(c.job.letter_distribution)),
        key('Board', show(c.job.layout)),
        more('Bingo bonus', show(c.job.bingo_bonus)),
        more('Sim cutoff', show(c.job.sim_cutoff)),
        more('Redundancy', `${c.job.redundancy}×`),
        more('Oldest MAGPIE', show(c.job.min_magpie_version))
      ]
    }
  ];
  if (c.games) {
    const g = c.games;
    const units = g.unit === 'pair' ? 'Pairs' : 'Games';
    const kind = g.unit === 'pair' ? 'Game pairs' : 'Games';
    // A job without a test stores the test's defaults, which it never reads:
    // shown, they would read as a test it runs.
    const test: JobSetting[] = g.sprt_enabled
      ? [
          key(`Cap (${g.unit}s)`, show(g.max_units)),
          key('Elo H0', show(g.elo_low)),
          key('Elo H1', show(g.elo_high)),
          more(`Fewest ${g.unit}s before the test is acted on`, show(g.min_units)),
          more('SPRT α', show(g.sprt_alpha)),
          more('SPRT β', show(g.sprt_beta))
        ]
      : [key(`${units} to play`, show(g.max_units)), key('SPRT', 'none')];
    groups.push({
      title: g.sprt_enabled ? `${kind} and the test` : kind,
      rows: [
        ...test,
        more(`${units} per task`, show(g.per_batch)),
        more('Records positions', show(g.capture_positions))
      ]
    });
  }
  if (c.opening_racks) {
    const o = c.opening_racks;
    groups.push({
      title: 'Opening racks',
      rows: [
        key('Racks in all', show(o.total_racks)),
        key('Rack size', show(o.rack_size)),
        more('Racks per task', show(o.racks_per_batch))
      ]
    });
  }
  if (c.leave_generation) {
    const l = c.leave_generation;
    groups.push({
      title: 'Leave generation',
      rows: [
        key('Generations', show(l.target_rack_counts.length)),
        key('Target per rack', l.target_rack_counts.map((t) => t.toLocaleString()).join(', ')),
        key('Games per task', show(l.num_iterations)),
        more('Racks per task', show(l.racks_per_task))
      ]
    });
  }
  return groups;
}

/** The groups' key rows alone, a group with none left out. */
export function keyGroups(groups: SettingGroup[]): SettingGroup[] {
  return groups
    .map((g) => ({ title: g.title, rows: g.rows.filter((r) => r.key) }))
    .filter((g) => g.rows.length > 0);
}

/** Every player setting, labelled, in the order a reader compares them. */
const PLAYER_ROWS: [keyof PlayerSettings, string][] = [
  ['lexicon', 'Lexicon'],
  ['leaves', 'Leaves'],
  ['win_pct', 'Win %'],
  ['num_plies', 'Plies'],
  ['max_iterations', 'Iterations (most)'],
  ['min_play_iterations', 'Iterations per play (fewest)'],
  ['stopping_pct', 'Stopping %'],
  ['time_limit_secs', 'Time limit (s)'],
  ['use_inference', 'Inference'],
  ['inference_margin', 'Inference margin'],
  ['threshold', 'Threshold'],
  ['sampling_rule', 'Sampling rule'],
  ['num_plays', 'Plays considered'],
  ['sort_strategy', 'Sort'],
  ['recorder_type', 'Recorder'],
  ['num_plays_recorded', 'Plays recorded'],
  ['num_plies_recorded', 'Plies recorded'],
  ['movegen_margin', 'Move-gen margin'],
  ['utility_w_winpct', 'Utility weight: win %'],
  ['utility_w_spread', 'Utility weight: spread'],
  ['utility_spread_scale', 'Utility spread scale'],
  ['use_wordmap', 'Wordmap'],
  ['use_rit', 'Rack info table']
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

const list = (values: number[] | null) => (values ? values.join(', ') : '—');

/**
 * The endgame and pre-endgame rows of the full table, after every other
 * setting: the two switches, then the pre-endgame's schedule.
 */
function solverRows(players: PlayerSettings[]): SettingRow[] {
  return [
    row('Endgame', players.map(endgameText)),
    row('Pre-endgame', players.map(preEndgameText)),
    row('PEG schedule', players.map((p) => list(p.peg_stage_top_k))),
    row('PEG stride', players.map((p) => show(p.peg_scenario_stride))),
    row('PEG opponent', players.map((p) => show(p.peg_opp_model))),
    row('Nested lookahead', players.map((p) => show(p.peg_nested))),
    row('Nested caps', players.map((p) => list(p.peg_nested_cand_caps))),
    row('Nested depth', players.map((p) => show(p.peg_nested_max_depth))),
    row('Nested strides', players.map((p) => list(p.peg_nested_strides)))
  ];
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
  'Win %',
  'Recorder',
  'Plays recorded',
  'Plies recorded',
  'Move-gen margin',
  // A leave game ends before the bag is small enough for either solver.
  ...SOLVER_ROWS
]);

/** An opening rack is analysed on an empty board, far from the endgame. */
const OPENING_RACK_UNUSED: ReadonlySet<string> = new Set(SOLVER_ROWS);

/** The player settings, by row label, that a job of `jobType` never reads. */
export function unusedPlayerSettings(jobType: JobType): ReadonlySet<string> {
  if (jobType === 'leave_generation') return LEAVE_UNUSED;
  if (jobType === 'opening_rack') return OPENING_RACK_UNUSED;
  return NONE;
}

function marked(rows: SettingRow[], unused: ReadonlySet<string>): SettingRow[] {
  return rows.map((r) => (unused.has(r.label) ? { ...r, unused: true } : r));
}

/**
 * What a reader compares configs by first -- the search in a few words, the
 * files, the plays considered and what is kept -- one value per player.
 * `playerRows` has every setting. Rows named in `unused` are marked so.
 */
export function keySettings(players: PlayerSettings[], unused = NONE): SettingRow[] {
  const solversUnused = unused.has('Endgame');
  const rows = [
    row('Search', players.map((p) => playerSummary(p, solversUnused))),
    row('Lexicon', players.map((p) => show(p.lexicon))),
    row('Leaves', players.map((p) => show(p.leaves)))
  ];
  // A static player has no win% model to name; a row of dashes says nothing.
  if (players.some((p) => p.win_pct)) rows.push(row('Win %', players.map((p) => show(p.win_pct))));
  rows.push(
    row('Plays considered', players.map((p) => show(p.num_plays))),
    row(
      'Recorder',
      players.map((p) => `${p.recorder_type}, ${show(p.num_plays_recorded)} play${p.num_plays_recorded === 1 ? '' : 's'} kept`)
    ),
    row('Wordmap', players.map((p) => show(p.use_wordmap))),
    row('Rack info table', players.map((p) => show(p.use_rit)))
  );
  // A player that solves nothing has no endgame to name: rows of "off" say
  // nothing.
  if (players.some(solves)) {
    rows.push(row('Endgame', players.map(endgameText)), row('Pre-endgame', players.map(preEndgameText)));
  }
  return marked(rows, unused);
}

/**
 * Every setting, one value per player, after the search in a few words: the
 * row a reader looks for first stays first when the table grows. Rows named in
 * `unused` are marked so.
 */
export function playerRows(players: PlayerSettings[], unused = NONE): SettingRow[] {
  const solversUnused = unused.has('Endgame');
  return marked(
    [
      row('Search', players.map((p) => playerSummary(p, solversUnused))),
      ...PLAYER_ROWS.map(([k, label]) => row(label, players.map((p) => show(p[k] as Value)))),
      ...solverRows(players)
    ],
    unused
  );
}

/** The players' searches in one line, for beside a job's lexicon and variant. */
export function playersLine(c: JobConfig): string {
  const solversUnused = unusedPlayerSettings(c.job.job_type).has('Endgame');
  return c.players.map((p) => playerSummary(p, solversUnused)).join(' vs ');
}
