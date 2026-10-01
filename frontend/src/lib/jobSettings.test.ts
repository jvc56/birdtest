import { describe, expect, it } from 'vitest';
import { jobGroups, keyGroups, keySettings, playerRows, playersLine, playerSummary, show, unusedPlayerSettings, type JobConfig, type PlayerSettings } from './jobSettings';

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

describe('F-SET-1 job settings', () => {
  it("says how each player searches in a few words", () => {
    expect(playerSummary(staticPlayer)).toBe('static, by equity');
    expect(playerSummary(simPlayer)).toBe('4-ply sim, 1,000 iterations, stops at 99%, inference');
    expect(playersLine(config)).toBe('static, by equity vs 4-ply sim, 1,000 iterations, stops at 99%, inference');
  });

  it('shows blanks, flags and numbers plainly', () => {
    expect(show(null)).toBe('—');
    expect(show(true)).toBe('yes');
    expect(show(5000)).toBe((5000).toLocaleString());
  });

  it("groups the job's settings and its type's", () => {
    const groups = jobGroups(config);
    expect(groups.map((g) => g.title)).toEqual(['Job', 'Game pairs and the test']);
    expect(groups[1].rows).toContainEqual({ label: 'Cap (pairs)', value: (5000).toLocaleString(), key: true });
  });

  it("leads with the job's key rows, and a group with none is left out", () => {
    const labels = (c: JobConfig) => keyGroups(jobGroups(c)).map((g) => [g.title, g.rows.map((r) => r.label)]);
    expect(labels(config)).toEqual([
      ['Job', ['Type', 'Variant', 'Letter distribution', 'Board']],
      ['Game pairs and the test', ['Cap (pairs)', 'Elo H0', 'Elo H1']]
    ]);
    expect(keyGroups(jobGroups(config))[0].rows[0].value).toBe('Game pairs');
    // Without a test: its target, and that there is none.
    expect(labels({ ...config, games: { ...config.games!, sprt_enabled: false } })[1]).toEqual([
      'Game pairs',
      ['Pairs to play', 'SPRT']
    ]);
    // Every row is still there for "All settings".
    const every = jobGroups(config).flatMap((g) => g.rows.map((r) => r.label));
    for (const label of ['Redundancy', 'Oldest MAGPIE', 'SPRT α', 'Pairs per task', 'Records positions']) {
      expect(every).toContain(label);
    }
    expect(keyGroups([{ title: 'Nothing key', rows: [{ label: 'a', value: 'b', key: false }] }])).toEqual([]);
  });

  it("shows a leave job's generations and each one's target, and leaves its lexicon to its player", () => {
    const leave: JobConfig = {
      job: { ...config.job, job_type: 'leave_generation' },
      leave_generation: { num_iterations: 10000, target_rack_counts: [100, 200, 5000], racks_per_task: 50 },
      players: [{ ...staticPlayer, role: 'player' }]
    };
    const group = jobGroups(leave).find((g) => g.title === 'Leave generation')!;
    expect(group.rows).toEqual([
      { label: 'Generations', value: '3', key: true },
      { label: 'Target per rack', value: `100, 200, ${(5000).toLocaleString()}`, key: true },
      { label: 'Games per task', value: (10000).toLocaleString(), key: true },
      { label: 'Racks per task', value: '50', key: false }
    ]);
    // The lexicon and wordmap are the player's rows, not the job's.
    const every = jobGroups(leave).flatMap((g) => g.rows.map((r) => r.label));
    expect(every).not.toContain('Lexicon');
    expect(every).not.toContain('Wordmap');
    expect(keySettings(leave.players).map((r) => r.label)).toEqual(
      expect.arrayContaining(['Lexicon', 'Wordmap'])
    );
  });

  it("marks the player settings a leave job never reads, and no other job's", () => {
    const unused = unusedPlayerSettings('leave_generation');
    const muted = (rows: { label: string; unused?: true }[]) => rows.filter((r) => r.unused).map((r) => r.label);
    expect(muted(keySettings([staticPlayer], unused))).toEqual(['Leaves', 'Recorder']);
    const solverRows = [
      'Endgame', 'Pre-endgame', 'PEG schedule', 'PEG stride', 'PEG opponent', 'Nested lookahead',
      'Nested caps', 'Nested depth', 'Nested strides'
    ];
    expect(muted(playerRows([staticPlayer], unused))).toEqual([
      'Leaves', 'Win %', 'Recorder', 'Plays recorded', 'Plies recorded', 'Move-gen margin',
      ...solverRows
    ]);
    // What it plays with is not muted: the lexicon, the search and the wordmap.
    for (const used of ['Search', 'Lexicon', 'Plies', 'Sort', 'Wordmap']) {
      expect(playerRows([staticPlayer], unused).find((r) => r.label === used)!.unused).toBeUndefined();
    }
    for (const type of ['games', 'game_pairs'] as const) {
      expect(muted(playerRows([staticPlayer], unusedPlayerSettings(type)))).toEqual([]);
    }
    // An opening rack never reaches the end of a game.
    expect(muted(playerRows([staticPlayer], unusedPlayerSettings('opening_rack')))).toEqual(solverRows);
    // Every label named is a row of the table, so none is silently unmatched.
    const labels = playerRows([simPlayer]).map((r) => r.label);
    for (const label of unused) expect(labels).toContain(label);
  });

  it('shows a job without a test its target, and none of the test it does not run', () => {
    const groups = jobGroups({ ...config, games: { ...config.games!, sprt_enabled: false } });
    expect(groups[1].title).toBe('Game pairs');
    const labels = groups[1].rows.map((r) => r.label);
    expect(groups[1].rows).toContainEqual({ label: 'Pairs to play', value: (5000).toLocaleString(), key: true });
    expect(groups[1].rows).toContainEqual({ label: 'SPRT', value: 'none', key: true });
    for (const hidden of ['SPRT α', 'SPRT β', 'Elo H0', 'Elo H1', 'Cap (pairs)']) {
      expect(labels).not.toContain(hidden);
    }
    expect(labels.some((label) => label.startsWith('Fewest'))).toBe(false);
  });

  it('puts the players side by side and marks what they differ in', () => {
    const rows = playerRows(config.players);
    const plies = rows.find((r) => r.label === 'Plies')!;
    expect(plies).toEqual({ label: 'Plies', values: ['0', '4'], differs: true });
    expect(rows.find((r) => r.label === 'Lexicon')!.differs).toBe(false);
    // Every setting a player config has is shown, after the search in a few words.
    expect(rows[0]).toEqual({ label: 'Search', values: config.players.map((p) => playerSummary(p)), differs: true });
    expect(rows).toHaveLength(33);
  });

  it('shows how a player solves the end of the game, where the job reaches it', () => {
    expect(playerSummary(solvingPlayer)).toBe('static, by equity · 6-ply endgame · PEG ≤2');
    expect(playerSummary({ ...solvingPlayer, peg_max_bag: 0 })).toBe('static, by equity · 6-ply endgame');
    // Where the job never reaches the end of a game, the summary leaves it out.
    expect(playerSummary(solvingPlayer, true)).toBe('static, by equity');
    const rows = playerRows([staticPlayer, solvingPlayer]);
    const value = (label: string) => rows.find((r) => r.label === label)!;
    expect(value('Endgame')).toEqual({ label: 'Endgame', values: ['off', '6-ply endgame'], differs: true });
    expect(value('Pre-endgame').values).toEqual(['off', 'bag ≤ 2']);
    expect(value('PEG schedule').values).toEqual(['—', '32, 16, 8, 4, 2']);
    expect(value('Nested strides').values).toEqual(['—', '1, 1, 5, 7']);
    // The pre-endgame is off without the endgame, whatever its bag says.
    expect(playerRows([{ ...solvingPlayer, endgame_plies: 0 }]).find((r) => r.label === 'Pre-endgame')!.values)
      .toEqual(['off']);
    // Key rows name the solving only when a player solves.
    const labels = (r: { label: string }[]) => r.map((x) => x.label);
    expect(labels(keySettings([staticPlayer]))).not.toContain('Endgame');
    expect(keySettings([staticPlayer, solvingPlayer])).toContainEqual({
      label: 'Pre-endgame', values: ['off', 'bag ≤ 2'], differs: true
    });
    // An opening-rack job's summary does not claim a solve it never runs.
    expect(
      playersLine({ ...config, job: { ...config.job, job_type: 'opening_rack' }, players: [solvingPlayer] })
    ).toBe('static, by equity');
  });

  it('leads with what tells configs apart, for one player or two', () => {
    const labels = (rows: { label: string }[]) => rows.map((r) => r.label);
    // A static player has no win% model to name.
    expect(labels(keySettings([staticPlayer]))).not.toContain('Win %');
    const sim = keySettings([simPlayer]);
    expect(sim[0]).toEqual({ label: 'Search', values: [playerSummary(simPlayer)], differs: false });
    expect(sim).toContainEqual({ label: 'Win %', values: ['winpct'], differs: false });
    expect(sim).toContainEqual({ label: 'Recorder', values: ['best, 10 plays kept'], differs: false });
    expect(sim).toContainEqual({ label: 'Rack info table', values: ['yes'], differs: false });
    // Side by side, a row either player has, with a dash for the other.
    const both = keySettings([staticPlayer, simPlayer]);
    expect(both).toContainEqual({ label: 'Win %', values: ['—', 'winpct'], differs: true });
    expect(both.find((r) => r.label === 'Leaves')!.differs).toBe(false);
    // Every key row is a row of the full table too, or toggling would drop it.
    const every = labels(playerRows([simPlayer]));
    for (const label of labels(sim)) expect(every).toContain(label);
  });
});
