import { describe, expect, it } from 'vitest';
import type { ComponentType, SvelteComponent } from 'svelte';
import type { PlayerSettings } from '$lib/jobSettings';
import PlayerSettingsTable from './PlayerSettingsTable.svelte';

// Rendered as the server would, to HTML: Vitest compiles a component for SSR
// in its Node environment, so the markup can be read without a DOM library.
type Rendered = { html: string };
const render = (props: Record<string, unknown>): string =>
  (PlayerSettingsTable as unknown as ComponentType<SvelteComponent> & { render: (p: object) => Rendered }).render(props)
    .html;

const base: PlayerSettings = {
  role: 'player 1', id: 'p1', name: 'static-equity', lexicon: 'CSW24', leaves: 'CSW24', win_pct: null,
  recorder_type: 'all', sort_strategy: 'equity', num_plies: 0, num_plies_recorded: 0, num_plays: 100,
  num_plays_recorded: 10, max_iterations: null, stopping_pct: null, use_inference: null, time_limit_secs: null,
  use_wordmap: true, use_rit: false, use_wit: false, min_play_iterations: null, threshold: null,
  sampling_rule: null, inference_margin: null, utility_w_winpct: null, utility_w_spread: null,
  utility_spread_scale: null, movegen_margin: 0, endgame_plies: 0, peg_max_bag: 0, peg_stage_top_k: null,
  peg_scenario_stride: null, peg_opp_model: null, peg_nested: null, peg_nested_cand_caps: null,
  peg_nested_max_depth: null, peg_nested_strides: null
};
// They differ in a key row (Sorted By) and in one only "All settings" lists (Word Info Table).
const other: PlayerSettings = { ...base, role: 'player 2', id: 'p2', name: 'static-score', sort_strategy: 'score', use_wit: true };

/** The `data-setting` ids in the `<tbody>` with this test id, in order; null when it is not rendered. */
function block(html: string, testid: string): string[] | null {
  const match = new RegExp(`<tbody[^>]*data-testid="${testid}"[^>]*>([\\s\\S]*?)</tbody>`).exec(html);
  return match ? [...match[1].matchAll(/data-setting="([^"]+)"/g)].map((m) => m[1]) : null;
}

describe('F-SET-2 the player settings table reads differences first', () => {
  it('puts the rows the players differ in in a block of their own, the shared ones folded', () => {
    const html = render({ players: [base, other] });
    expect(block(html, 'setting-differences')).toEqual(['sort_strategy', 'use_wit']);
    expect(html).toContain('Differences (2)');
    expect(block(html, 'shared-settings')).toBeNull();
    // The key rows they share: Lexicon, Leaves, Move Recorder, Moves Generated,
    // Plies, Uses Inference, Uses Preendgame, Uses Endgame.
    expect(html).toContain('Show 8 shared settings');
    expect(html).not.toContain('in bold');
    // Each player headed by the colour its moves are drawn in.
    expect(html.match(/data-testid="player-color"/g)).toHaveLength(2);
  });

  it('opens the shared rows when every setting is asked for', () => {
    const html = render({ players: [base, other], all: true });
    expect(block(html, 'setting-differences')).toEqual(['sort_strategy', 'use_wit']);
    const shared = block(html, 'shared-settings')!;
    expect(shared).toEqual(expect.arrayContaining(['lexicon', 'movegen_margin', 'use_wordmap']));
    expect(shared).not.toContain('sort_strategy');
    expect(html).toContain('Hide shared settings');
  });

  it('shows one player, or two alike, open with no differences block', () => {
    for (const players of [[{ ...base, role: undefined }], [base, { ...base, role: 'player 2' }]]) {
      const html = render({ players });
      expect(block(html, 'setting-differences')).toBeNull();
      expect(block(html, 'shared-settings')![0]).toBe('lexicon');
      expect(html).not.toContain('shared setting');
    }
    // A config read on its own has no colour: nothing is drawn in it.
    expect(render({ players: [{ ...base, role: undefined }] })).not.toContain('player-color');
  });
});
