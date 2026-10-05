import { describe, expect, it } from 'vitest';
import { jobSettings, keySettings, playerRows, playersLine, playerSummary, show, unusedPlayerSettings, type JobConfig, type PlayerSettings } from './jobSettings';

const staticPlayer: PlayerSettings = {
  role: 'player 1', id: 'p1', name: 'static-NWL23', lexicon: 'NWL23', leaves: 'NWL23', win_pct: null,
  recorder_type: 'best', sort_strategy: 'equity', num_plies: 0, num_plies_recorded: 2,
  num_plays: 100, num_plays_recorded: 10, max_iterations: null, stopping_pct: null,
  use_inference: null, time_limit_secs: null, use_wordmap: true, use_rit: true,
  use_wit: false,
  min_play_iterations: null, threshold: null, sampling_rule: null, inference_margin: null,
  utility_w_winpct: null, utility_w_spread: null, utility_spread_scale: null, movegen_margin: 5,
  endgame_plies: 0, peg_max_bag: 0, peg_stage_top_k: null, peg_scenario_stride: null,
  peg_opp_model: null, peg_nested: null, peg_nested_cand_caps: null, peg_nested_max_depth: null,
  peg_nested_strides: null
};
const solvingPlayer: PlayerSettings = {
  ...staticPlayer, role: 'player 2', id: 'p3', name: 'solver-NWL23',
  endgame_plies: 6, peg_max_bag: 2, peg_stage_top_k: [32, 16, 8, 4, 2], peg_scenario_stride: 1,
  peg_opp_model: 'rational', peg_nested: true, peg_nested_cand_caps: [8, 4, 2],
  peg_nested_max_depth: 1, peg_nested_strides: [1, 1, 5, 7]
};
const simPlayer: PlayerSettings = {
  ...staticPlayer, role: 'player 2', id: 'p2', name: 'simmer-NWL23-4ply', win_pct: 'winpct',
  num_plies: 4, max_iterations: 1000, stopping_pct: 99, use_inference: true,
  time_limit_secs: 30, min_play_iterations: 100, threshold: 'gk16', sampling_rule: 'round_robin',
  inference_margin: 0, utility_w_winpct: 1, utility_w_spread: 0, utility_spread_scale: 1
};
const config: JobConfig = {
  job: {
    id: 'j', name: 'n', job_type: 'game_pairs', variant: 'classic', letter_distribution: 'english',
    layout: 'standard15', bingo_bonus: 50, sim_cutoff: 0, min_magpie_version: '0.1.1'
  },
  games: {
    unit: 'pair', per_batch: 1, test_enabled: true, min_units: 100, max_units: 5000,
    confidence_pct: 95, capture_positions: false,
    capture_first_divergence: false
  },
  players: [staticPlayer, simPlayer]
};

const labels = (rows: { label: string }[]) => rows.map((r) => r.label);
const muted = (rows: { label: string; unused?: true }[]) => rows.filter((r) => r.unused).map((r) => r.label);
const byLabel = <T extends { label: string }>(rows: T[], label: string): T => rows.find((r) => r.label === label)!;
/** The player settings always shown, in order. */
const KEY_LABELS = [
  'Lexicon', 'Leaves', 'Sorted By', 'Move Recorder', 'Moves Generated', 'Plies', 'Uses Inference',
  'Uses Preendgame', 'Uses Endgame'
];
const SOLVER_LABELS = [
  'Uses Endgame', 'Uses Preendgame', 'PEG Schedule', 'PEG Stride', 'PEG Opponent', 'Nested Lookahead',
  'Nested Caps', 'Nested Depth', 'Nested Strides'
];
const opening: JobConfig = {
  job: { ...config.job, job_type: 'opening_rack' },
  opening_racks: {
    racks_per_batch: 500, rack_size: 7, total_racks: 3199724,
    consensus_pct: 80, min_results_per_rack: 3, max_results_per_rack: 7
  },
  players: [{ ...simPlayer, role: 'player' }]
};
const leave: JobConfig = {
  job: { ...config.job, job_type: 'leave_generation' },
  leave_generation: { num_iterations: 10000, target_rack_counts: [100, 200, 5000], racks_per_task: 50 },
  players: [{ ...staticPlayer, role: 'player' }]
};

describe('F-SET-1 job settings', () => {
  it('says how each player searches in a few words', () => {
    expect(playerSummary(staticPlayer)).toBe('static, by equity');
    expect(playerSummary(simPlayer)).toBe('4-ply sim, 1,000 iterations, stops at 99%, inference');
    expect(playersLine(config)).toBe('static, by equity vs 4-ply sim, 1,000 iterations, stops at 99%, inference');
    // An opening rack has no previous play to infer from: the summary does
    // not claim inference it never runs.
    expect(playersLine(opening)).toBe('4-ply sim, 1,000 iterations, stops at 99%');
  });

  it('shows blanks, flags and numbers plainly', () => {
    expect(show(null)).toBe('—');
    expect(show(true)).toBe('yes');
    expect(show(false)).toBe('no');
    expect(show(5000)).toBe((5000).toLocaleString());
  });

  it("lists every one of a games job's settings in one order, in Title Case", () => {
    const rows = jobSettings(config);
    expect(labels(rows)).toEqual([
      'Type', 'Variant', 'Letter Distribution', 'Board', 'Bingo Bonus', 'Maximum Pairs',
      'Significance Test', 'Position Recorder', 'Sim Cutoff', 'Minimum Pairs', 'Pairs Per Task',
      'Oldest MAGPIE'
    ]);
    expect(rows[0].value).toBe('Game Pairs');
    // The test and its confidence in one row.
    expect(byLabel(rows, 'Significance Test').value).toBe('yes (95%)');
    expect(byLabel(rows, 'Bingo Bonus')).toEqual({ id: 'bingo_bonus', label: 'Bingo Bonus', value: '50' });
    // A games job counts games.
    const games = jobSettings({ ...config, games: { ...config.games!, unit: 'game' } });
    expect(labels(games)).toEqual(expect.arrayContaining(['Maximum Games', 'Minimum Games', 'Games Per Task']));
  });

  it('capitalises every job type', () => {
    const type = (job_type: JobConfig['job']['job_type']) => jobSettings({ ...config, job: { ...config.job, job_type } })[0].value;
    expect(type('opening_rack')).toBe('Opening Rack Analysis');
    expect(type('games')).toBe('Games');
    expect(type('game_pairs')).toBe('Game Pairs');
    expect(type('leave_generation')).toBe('Leave Generation');
  });

  it('says yes or no to recording positions, and when a pairs job keeps only first divergences', () => {
    const value = (games: Partial<NonNullable<JobConfig['games']>>) =>
      byLabel(jobSettings({ ...config, games: { ...config.games!, ...games } }), 'Position Recorder').value;
    expect(value({})).toBe('no');
    expect(value({ capture_positions: true })).toBe('yes');
    expect(value({ capture_positions: true, capture_first_divergence: true })).toBe('yes (first divergences)');
  });

  it('shows a job without a test its target, and none of the test it does not run', () => {
    const rows = jobSettings({ ...config, games: { ...config.games!, test_enabled: false } });
    expect(byLabel(rows, 'Pairs To Play').value).toBe((5000).toLocaleString());
    expect(byLabel(rows, 'Significance Test').value).toBe('no');
    for (const hidden of ['Maximum Pairs', 'Minimum Pairs']) expect(labels(rows)).not.toContain(hidden);
  });

  it("lists an opening-rack job's analyses per rack, and a leave job's generations and no sim cutoff", () => {
    expect(labels(jobSettings(opening))).toEqual([
      'Type', 'Variant', 'Letter Distribution', 'Board', 'Bingo Bonus', 'Minimum Analyses Per Rack',
      'Maximum Analyses Per Rack', 'Consensus %', 'Sim Cutoff', 'Racks Per Task', 'Oldest MAGPIE'
    ]);
    const value = (label: string, o: Partial<NonNullable<JobConfig['opening_racks']>> = {}) =>
      byLabel(jobSettings({ ...opening, opening_racks: { ...opening.opening_racks!, ...o } }), label).value;
    expect(value('Minimum Analyses Per Rack')).toBe('3');
    expect(value('Maximum Analyses Per Rack')).toBe('7');
    expect(value('Consensus %')).toBe('80%');
    // One analysis per rack seeks no agreement.
    expect(value('Consensus %', { min_results_per_rack: 1, max_results_per_rack: 1 })).toBe('—');
    const rows = jobSettings(leave);
    expect(labels(rows)).toEqual([
      'Type', 'Variant', 'Letter Distribution', 'Board', 'Bingo Bonus', 'Generations', 'Target Per Rack',
      'Games Per Task', 'Racks Per Task', 'Oldest MAGPIE'
    ]);
    expect(byLabel(rows, 'Target Per Rack').value).toBe(`100 → 200 → ${(5000).toLocaleString()}`);
    // The lexicon and wordmap are the player's rows, not the job's.
    expect(labels(rows)).not.toContain('Lexicon');
    expect(labels(playerRows(leave.players))).toEqual(expect.arrayContaining(['Lexicon', 'Wordmap']));
  });

  it('marks the player settings each job never reads', () => {
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings(leave)))).toEqual([
      'Leaves', 'Move Recorder', 'Uses Preendgame', 'Uses Endgame', 'Moves Recorded', 'Plies Recorded',
      'Movegen Margin', ...SOLVER_LABELS.slice(2)
    ]);
    // A leave job's simmer would show its win% model muted too (job creation
    // refuses one, but the set names it).
    expect(unusedPlayerSettings(leave).has('win_pct')).toBe(true);
    // What it plays with is not muted.
    for (const used of ['Lexicon', 'Plies', 'Sorted By', 'Wordmap']) {
      expect(byLabel(playerRows([staticPlayer], unusedPlayerSettings(leave)), used).unused).toBeUndefined();
    }
    // An opening rack infers nothing and never reaches the end of a game.
    expect(muted(playerRows([simPlayer], unusedPlayerSettings(opening)))).toEqual([
      'Uses Inference', 'Uses Preendgame', 'Uses Endgame', 'Inference Margin', ...SOLVER_LABELS.slice(2)
    ]);
    // A games job reads what is recorded only when it records positions.
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings(config)))).toEqual(['Moves Recorded', 'Plies Recorded']);
    const capturing = { ...config, games: { ...config.games!, capture_positions: true } };
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings(capturing)))).toEqual([]);
    // Every id named is a row of the full table, so none is silently unmatched.
    const every = playerRows([simPlayer, solvingPlayer]).map((r) => r.id);
    for (const job of [leave, opening, config]) {
      for (const id of unusedPlayerSettings(job)) expect(every).toContain(id);
    }
  });

  it('lists every player setting in one order, the key rows first, in Title Case', () => {
    expect(labels(playerRows([simPlayer, solvingPlayer]))).toEqual([
      ...KEY_LABELS, 'Win % Model', 'Maximum Total Iterations', 'Stopping %', 'Moves Recorded',
      'Plies Recorded', 'Minimum Iterations per Play', 'Stopping Threshold Rule', 'Sampling Rule',
      'Inference Margin', 'Win % Utility Weight', 'Spread Utility Weight', 'Spread Utility Scale',
      'Time Limit (Seconds)', 'Movegen Margin', ...SOLVER_LABELS.slice(2), 'Wordmap', 'Rack Info Table',
      'Word Info Table'
    ]);
    expect(labels(keySettings([simPlayer, solvingPlayer]))).toEqual(KEY_LABELS);
    // Every key row is a row of the full table too, or toggling would drop it.
    const every = labels(playerRows([simPlayer]));
    for (const label of labels(keySettings([simPlayer]))) expect(every).toContain(label);
  });

  it('leaves out the simulation rows when nobody simulates, and always shows the key rows', () => {
    const rows = labels(playerRows([staticPlayer]));
    for (const sim of ['Win % Model', 'Maximum Total Iterations', 'Stopping Threshold Rule', 'Time Limit (Seconds)', 'Inference Margin']) {
      expect(rows).not.toContain(sim);
    }
    // The key rows whatever the player: a static one infers nothing ("—")
    // and solves nothing ("no").
    expect(labels(keySettings([staticPlayer]))).toEqual(KEY_LABELS);
    expect(byLabel(keySettings([staticPlayer]), 'Uses Inference').values).toEqual(['—']);
    expect(byLabel(keySettings([staticPlayer]), 'Uses Endgame').values).toEqual(['no']);
    // Beside a simmer, a static player shows a dash in them.
    expect(byLabel(playerRows([staticPlayer, simPlayer]), 'Win % Model')).toEqual({
      id: 'win_pct', label: 'Win % Model', values: ['—', 'winpct'], differs: true
    });
  });

  it('shows a dash where a setting does not apply to the player', () => {
    // A static player records no plies, whatever its config says.
    expect(byLabel(playerRows([staticPlayer, simPlayer]), 'Plies Recorded').values).toEqual(['—', '2']);
    // The margin bounds only a recorder that keeps moves by equity.
    const margin = (recorder_type: string) =>
      byLabel(playerRows([{ ...staticPlayer, recorder_type }]), 'Movegen Margin').values;
    expect(margin('best')).toEqual(['—']);
    expect(margin('all')).toEqual(['—']);
    expect(margin('equity')).toEqual(['5']);
  });

  it('puts the players side by side and marks what they differ in', () => {
    const rows = playerRows(config.players);
    expect(byLabel(rows, 'Plies')).toEqual({ id: 'num_plies', label: 'Plies', values: ['0', '4'], differs: true });
    expect(byLabel(rows, 'Lexicon').differs).toBe(false);
  });

  it('shows how a player solves the end of the game, where the job reaches it', () => {
    expect(playerSummary(solvingPlayer)).toBe('static, by equity · 6-ply endgame · PEG ≤2');
    expect(playerSummary({ ...solvingPlayer, peg_max_bag: 0 })).toBe('static, by equity · 6-ply endgame');
    // Where the job never reaches the end of a game, the summary leaves it out.
    expect(playerSummary(solvingPlayer, unusedPlayerSettings(opening))).toBe('static, by equity');
    const rows = playerRows([staticPlayer, solvingPlayer]);
    expect(byLabel(rows, 'Uses Endgame')).toEqual({
      id: 'endgame', label: 'Uses Endgame', values: ['no', 'yes (6 plies)'], differs: true
    });
    expect(byLabel(rows, 'Uses Preendgame').values).toEqual(['no', 'yes (bag ≤ 2)']);
    expect(byLabel(rows, 'PEG Schedule').values).toEqual(['—', '32, 16, 8, 4, 2']);
    expect(byLabel(rows, 'Nested Strides').values).toEqual(['—', '1, 1, 5, 7']);
    // The pre-endgame is off without the endgame, whatever its bag says.
    expect(byLabel(playerRows([{ ...solvingPlayer, endgame_plies: 0 }]), 'Uses Preendgame').values).toEqual(['no']);
    expect(keySettings([staticPlayer, solvingPlayer])).toContainEqual({
      id: 'pre_endgame', label: 'Uses Preendgame', values: ['no', 'yes (bag ≤ 2)'], differs: true
    });
  });
});
