import { describe, expect, it } from 'vitest';
import { jobSettings, keySettings, playerRows, playersLine, playerSummary, show, unusedPlayerSettings, type JobConfig, type PlayerSettings } from './jobSettings';

const staticPlayer: PlayerSettings = {
  role: 'player 1', id: 'p1', name: 'static-NWL23', lexicon: 'NWL23', leaves: 'NWL23', win_pct: null,
  recorder_type: 'best', sort_strategy: 'equity', num_plies: 0, num_plies_recorded: 2,
  num_plays: 100, num_plays_recorded: 10, max_iterations: null, stopping_pct: null,
  use_inference: null, time_limit_secs: null, use_wordmap: true, use_rit: true,
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
    layout: 'standard15', bingo_bonus: 50, sim_cutoff: 0, redundancy: 1, min_magpie_version: '0.1.1'
  },
  games: {
    unit: 'pair', per_batch: 1, sprt_enabled: true, min_units: 100, max_units: 5000,
    sprt_alpha: 0.05, sprt_beta: 0.05, elo_low: -10, elo_high: 10, capture_positions: false
  },
  players: [staticPlayer, simPlayer]
};

const labels = (rows: { label: string }[]) => rows.map((r) => r.label);
const keyLabels = (rows: { label: string; key: boolean }[]) => rows.filter((r) => r.key).map((r) => r.label);
const muted = (rows: { label: string; unused?: true }[]) => rows.filter((r) => r.unused).map((r) => r.label);
const SOLVER_ROWS = [
  'Endgame', 'Pre-endgame', 'PEG schedule', 'PEG stride', 'PEG opponent', 'Nested lookahead',
  'Nested caps', 'Nested depth', 'Nested strides'
];
const opening: JobConfig = {
  job: { ...config.job, job_type: 'opening_rack' },
  opening_racks: { racks_per_batch: 500, rack_size: 7, total_racks: 3199724 },
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
    expect(show(5000)).toBe((5000).toLocaleString());
  });

  it("lists a games job's settings in one order, the key rows first", () => {
    const rows = jobSettings(config);
    expect(labels(rows)).toEqual([
      'Type', 'Variant', 'Letter distribution', 'Board', 'Bingo bonus', 'Cap (pairs)', 'SPRT',
      'Records positions', 'Sim cutoff', 'Fewest pairs before the test is acted on', 'SPRT α', 'SPRT β',
      'Pairs per task', 'Redundancy', 'Oldest MAGPIE'
    ]);
    expect(keyLabels(rows)).toEqual([
      'Type', 'Variant', 'Letter distribution', 'Board', 'Bingo bonus', 'Cap (pairs)', 'SPRT',
      'Records positions'
    ]);
    expect(rows[0].value).toBe('Game pairs');
    expect(rows).toContainEqual({ label: 'SPRT', value: 'Elo -10 → 10', key: true });
    expect(rows).toContainEqual({ label: 'Bingo bonus', value: '50', key: true });
  });

  it('shows a job without a test its target, and none of the test it does not run', () => {
    const rows = jobSettings({ ...config, games: { ...config.games!, sprt_enabled: false } });
    expect(rows).toContainEqual({ label: 'Pairs to play', value: (5000).toLocaleString(), key: true });
    expect(rows).toContainEqual({ label: 'SPRT', value: 'none', key: true });
    for (const hidden of ['SPRT α', 'SPRT β', 'Cap (pairs)']) expect(labels(rows)).not.toContain(hidden);
    expect(labels(rows).some((label) => label.startsWith('Fewest'))).toBe(false);
  });

  it("lists an opening-rack job's racks, and a leave job's generations and no sim cutoff", () => {
    expect(labels(jobSettings(opening))).toEqual([
      'Type', 'Variant', 'Letter distribution', 'Board', 'Bingo bonus', 'Racks in all', 'Rack size',
      'Sim cutoff', 'Racks per task', 'Redundancy', 'Oldest MAGPIE'
    ]);
    const rows = jobSettings(leave);
    expect(labels(rows)).toEqual([
      'Type', 'Variant', 'Letter distribution', 'Board', 'Bingo bonus', 'Generations', 'Target per rack',
      'Games per task', 'Racks per task', 'Redundancy', 'Oldest MAGPIE'
    ]);
    expect(keyLabels(rows)).toEqual([
      'Type', 'Variant', 'Letter distribution', 'Board', 'Bingo bonus', 'Generations', 'Target per rack'
    ]);
    expect(rows).toContainEqual({ label: 'Target per rack', value: `100, 200, ${(5000).toLocaleString()}`, key: true });
    // The lexicon and wordmap are the player's rows, not the job's.
    expect(labels(rows)).not.toContain('Lexicon');
    expect(labels(playerRows(leave.players))).toEqual(expect.arrayContaining(['Lexicon', 'Wordmap']));
  });

  it('marks the player settings each job never reads', () => {
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings(leave)))).toEqual([
      'Leaves', 'Recorder', 'Plays recorded', 'Endgame', 'Pre-endgame', 'Plies recorded',
      'Move-gen margin', ...SOLVER_ROWS.slice(2)
    ]);
    // A leave job's simmer would show its win% model muted too (job creation
    // refuses one, but the set names it).
    expect(unusedPlayerSettings(leave).has('Win % model')).toBe(true);
    // What it plays with is not muted.
    for (const used of ['Lexicon', 'Plies', 'Sort', 'Wordmap']) {
      expect(playerRows([staticPlayer], unusedPlayerSettings(leave)).find((r) => r.label === used)!.unused).toBeUndefined();
    }
    // An opening rack infers nothing and never reaches the end of a game.
    expect(muted(playerRows([simPlayer], unusedPlayerSettings(opening)))).toEqual([
      'Inference', 'Endgame', 'Pre-endgame', 'Inference margin', ...SOLVER_ROWS.slice(2)
    ]);
    // A games job reads what is recorded only when it records positions.
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings(config)))).toEqual(['Plays recorded', 'Plies recorded']);
    const capturing = { ...config, games: { ...config.games!, capture_positions: true } };
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings(capturing)))).toEqual([]);
    // Every label named is a row of the full table, so none is silently unmatched.
    const every = labels(playerRows([simPlayer, solvingPlayer]));
    for (const job of [leave, opening, config]) {
      for (const label of unusedPlayerSettings(job)) expect(every).toContain(label);
    }
  });

  it('lists every player setting in one order, the key rows first', () => {
    expect(labels(playerRows([simPlayer, solvingPlayer]))).toEqual([
      'Lexicon', 'Leaves', 'Plies', 'Plays considered', 'Sort', 'Win % model', 'Iterations (most)',
      'Stopping %', 'Inference', 'Recorder', 'Plays recorded', 'Endgame', 'Pre-endgame', 'Plies recorded',
      'Iterations per play (fewest)', 'Threshold', 'Sampling rule', 'Inference margin',
      'Utility weight: win %', 'Utility weight: spread', 'Utility spread scale', 'Time limit (s)',
      'Move-gen margin', ...SOLVER_ROWS.slice(2), 'Wordmap', 'Rack info table'
    ]);
    expect(labels(keySettings([simPlayer, solvingPlayer]))).toEqual([
      'Lexicon', 'Leaves', 'Plies', 'Plays considered', 'Sort', 'Win % model', 'Iterations (most)',
      'Stopping %', 'Inference', 'Recorder', 'Plays recorded', 'Endgame', 'Pre-endgame'
    ]);
    // Every key row is a row of the full table too, or toggling would drop it.
    const every = labels(playerRows([simPlayer]));
    for (const label of labels(keySettings([simPlayer]))) expect(every).toContain(label);
  });

  it('leaves out the simulation rows when nobody simulates, and the solving ones from the key rows when nobody solves', () => {
    const rows = labels(playerRows([staticPlayer]));
    for (const sim of ['Win % model', 'Iterations (most)', 'Threshold', 'Time limit (s)', 'Inference margin']) {
      expect(rows).not.toContain(sim);
    }
    // The endgame rows stay in the full table, "off".
    expect(playerRows([staticPlayer]).find((r) => r.label === 'Endgame')!.values).toEqual(['off']);
    expect(labels(keySettings([staticPlayer]))).toEqual([
      'Lexicon', 'Leaves', 'Plies', 'Plays considered', 'Sort', 'Recorder', 'Plays recorded'
    ]);
    // Beside a simmer, a static player shows a dash in them.
    expect(playerRows([staticPlayer, simPlayer]).find((r) => r.label === 'Win % model')).toEqual({
      label: 'Win % model', values: ['—', 'winpct'], differs: true
    });
  });

  it('puts the players side by side and marks what they differ in', () => {
    const rows = playerRows(config.players);
    expect(rows.find((r) => r.label === 'Plies')).toEqual({ label: 'Plies', values: ['0', '4'], differs: true });
    expect(rows.find((r) => r.label === 'Lexicon')!.differs).toBe(false);
  });

  it('shows how a player solves the end of the game, where the job reaches it', () => {
    expect(playerSummary(solvingPlayer)).toBe('static, by equity · 6-ply endgame · PEG ≤2');
    expect(playerSummary({ ...solvingPlayer, peg_max_bag: 0 })).toBe('static, by equity · 6-ply endgame');
    // Where the job never reaches the end of a game, the summary leaves it out.
    expect(playerSummary(solvingPlayer, unusedPlayerSettings(opening))).toBe('static, by equity');
    const rows = playerRows([staticPlayer, solvingPlayer]);
    const value = (label: string) => rows.find((r) => r.label === label)!;
    expect(value('Endgame')).toEqual({ label: 'Endgame', values: ['off', '6-ply endgame'], differs: true });
    expect(value('Pre-endgame').values).toEqual(['off', 'bag ≤ 2']);
    expect(value('PEG schedule').values).toEqual(['—', '32, 16, 8, 4, 2']);
    expect(value('Nested strides').values).toEqual(['—', '1, 1, 5, 7']);
    // The pre-endgame is off without the endgame, whatever its bag says.
    expect(playerRows([{ ...solvingPlayer, endgame_plies: 0 }]).find((r) => r.label === 'Pre-endgame')!.values)
      .toEqual(['off']);
    expect(keySettings([staticPlayer, solvingPlayer])).toContainEqual({
      label: 'Pre-endgame', values: ['off', 'bag ≤ 2'], differs: true
    });
  });
});
