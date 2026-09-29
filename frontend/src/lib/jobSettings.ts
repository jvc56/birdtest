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
  leave_generation?: {
    lexicon: string;
    num_iterations: number;
    generation_count: number;
    target_rack_count: number;
    racks_per_task: number;
    use_wordmap: boolean;
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

/** How a player searches, in a few words: what tells two configs apart. */
export function playerSummary(p: PlayerSettings): string {
  if (p.num_plies === 0) return `static, by ${p.sort_strategy}`;
  const parts = [`${p.num_plies}-ply sim`];
  if (p.max_iterations !== null) parts.push(`${p.max_iterations.toLocaleString()} iterations`);
  if (p.stopping_pct !== null) parts.push(`stops at ${p.stopping_pct}%`);
  if (p.use_inference) parts.push('inference');
  return parts.join(', ');
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
 * opening-rack job's racks and their size; a leave job's lexicon, iterations,
 * generations and target. The rest -- batch sizes, bingo bonus, sim cutoff,
 * redundancy, the oldest MAGPIE, the test's error rates and minimum, whether
 * positions are recorded, the wordmap -- is under "All settings".
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
        key('Lexicon', show(l.lexicon)),
        key('Iterations', show(l.num_iterations)),
        key('Generations', show(l.generation_count)),
        key('Target racks', show(l.target_rack_count)),
        more('Racks per task', show(l.racks_per_task)),
        more('Wordmap', show(l.use_wordmap))
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
}

function row(label: string, values: string[]): SettingRow {
  return { label, values, differs: new Set(values).size > 1 };
}

/**
 * What a reader compares configs by first -- the search in a few words, the
 * files, the plays considered and what is kept -- one value per player.
 * `playerRows` has every setting.
 */
export function keySettings(players: PlayerSettings[]): SettingRow[] {
  const rows = [
    row('Search', players.map(playerSummary)),
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
  return rows;
}

/**
 * Every setting, one value per player, after the search in a few words: the
 * row a reader looks for first stays first when the table grows.
 */
export function playerRows(players: PlayerSettings[]): SettingRow[] {
  return [
    row('Search', players.map(playerSummary)),
    ...PLAYER_ROWS.map(([k, label]) => row(label, players.map((p) => show(p[k] as Value))))
  ];
}

/** The players' searches in one line, for beside a job's lexicon and variant. */
export function playersLine(c: JobConfig): string {
  return c.players.map(playerSummary).join(' vs ');
}
