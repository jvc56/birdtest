/**
 * A job's configuration (`GET /api/jobs/:id/config`) as the job page shows it:
 * a line naming what matters most -- each player's search -- and every
 * setting, grouped, for whoever wants them all.
 */
import type { JobType } from '$lib/api';

export interface PlayerSettings {
  role: string;
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

export interface SettingGroup {
  title: string;
  rows: [string, string][];
}

/** The job's own settings and its type's, grouped and labelled. */
export function jobGroups(c: JobConfig): SettingGroup[] {
  const groups: SettingGroup[] = [
    {
      title: 'Job',
      rows: [
        ['Variant', show(c.job.variant)],
        ['Letter distribution', show(c.job.letter_distribution)],
        ['Board', show(c.job.layout)],
        ['Bingo bonus', show(c.job.bingo_bonus)],
        ['Sim cutoff', show(c.job.sim_cutoff)],
        ['Redundancy', `${c.job.redundancy}×`],
        ['Oldest MAGPIE', show(c.job.min_magpie_version)]
      ]
    }
  ];
  if (c.games) {
    const g = c.games;
    groups.push({
      title: g.unit === 'pair' ? 'Game pairs and the test' : 'Games and the test',
      rows: [
        [`${g.unit === 'pair' ? 'Pairs' : 'Games'} per task`, show(g.per_batch)],
        [`Fewest ${g.unit}s before the test is acted on`, show(g.min_units)],
        [`Cap (${g.unit}s)`, show(g.max_units)],
        ['SPRT α', show(g.sprt_alpha)],
        ['SPRT β', show(g.sprt_beta)],
        ['Elo H0', show(g.elo_low)],
        ['Elo H1', show(g.elo_high)],
        ['Records positions', show(g.capture_positions)]
      ]
    });
  }
  if (c.opening_racks) {
    const o = c.opening_racks;
    groups.push({
      title: 'Opening racks',
      rows: [
        ['Racks per task', show(o.racks_per_batch)],
        ['Rack size', show(o.rack_size)],
        ['Racks in all', show(o.total_racks)]
      ]
    });
  }
  if (c.leave_generation) {
    const l = c.leave_generation;
    groups.push({
      title: 'Leave generation',
      rows: [
        ['Lexicon', show(l.lexicon)],
        ['Iterations', show(l.num_iterations)],
        ['Generations', show(l.generation_count)],
        ['Target racks', show(l.target_rack_count)],
        ['Racks per task', show(l.racks_per_task)],
        ['Wordmap', show(l.use_wordmap)]
      ]
    });
  }
  return groups;
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

/**
 * The players' settings side by side: one row per setting, one value per
 * player, and whether the players differ in it -- which is usually the
 * point of the job.
 */
export function playerRows(players: PlayerSettings[]): { label: string; values: string[]; differs: boolean }[] {
  return PLAYER_ROWS.map(([key, label]) => {
    const values = players.map((p) => show(p[key] as Value));
    return { label, values, differs: new Set(values).size > 1 };
  });
}

/** The players' searches in one line, for beside a job's lexicon and variant. */
export function playersLine(c: JobConfig): string {
  return c.players.map(playerSummary).join(' vs ');
}
