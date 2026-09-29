import { describe, expect, it } from 'vitest';
import { jobGroups, keyGroups, keySettings, playerRows, playersLine, playerSummary, show, type JobConfig, type PlayerSettings } from './jobSettings';

const staticPlayer: PlayerSettings = {
  role: 'player 1', id: 'p1', name: 'static-NWL23', lexicon: 'NWL23', leaves: 'NWL23', win_pct: null,
  recorder_type: 'best', sort_strategy: 'equity', num_plies: 0, num_plies_recorded: 2,
  num_plays: 100, num_plays_recorded: 10, max_iterations: null, stopping_pct: null,
  use_inference: null, time_limit_secs: null, use_wordmap: true, use_rit: true,
  min_play_iterations: null, threshold: null, sampling_rule: null, inference_margin: null,
  utility_w_winpct: null, utility_w_spread: null, utility_spread_scale: null, movegen_margin: 5
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
    expect(rows[0]).toEqual({ label: 'Search', values: config.players.map(playerSummary), differs: true });
    expect(rows).toHaveLength(24);
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
