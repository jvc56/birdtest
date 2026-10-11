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

/** The `data-setting` ids of the rows listed, in order; null when no table is rendered. */
function rows(html: string): string[] | null {
  const match = /<tbody[^>]*data-testid="setting-rows"[^>]*>([\s\S]*?)<\/tbody>/.exec(html);
  return match ? [...match[1].matchAll(/data-setting="([^"]+)"/g)].map((m) => m[1]) : null;
}

describe('F-SET-2 the player settings table lists differences, key rows or every setting', () => {
  it('lists only what two players differ in, key or not, as ordinary rows', () => {
    const html = render({ players: [base, other], mode: 'differences' });
    expect(rows(html)).toEqual(['sort_strategy', 'use_wit']);
    // No group header, no tint, no fold: the rows are the table.
    expect(html).not.toContain('Differences (');
    expect(html).not.toContain('bg-warning');
    expect(html).not.toContain('shared setting');
    // Each player headed by the colour its moves are drawn in.
    expect(html.match(/data-testid="player-color"/g)).toHaveLength(2);
  });

  it('lists every setting, the differing ones among them in order, when all are asked for', () => {
    const listed = rows(render({ players: [base, other], mode: 'all' }))!;
    expect(listed).toEqual(expect.arrayContaining(['lexicon', 'sort_strategy', 'use_wit', 'movegen_margin', 'use_wordmap']));
    expect(listed[0]).toBe('lexicon');
  });

  it('says two players alike are identical rather than show an empty table', () => {
    const html = render({ players: [base, { ...base, role: 'player 2' }], mode: 'differences' });
    expect(rows(html)).toBeNull();
    expect(html).toContain('settings are identical');
  });

  it('lists one player\'s key rows, with no colour', () => {
    const html = render({ players: [{ ...base, role: undefined }], mode: 'key' });
    expect(rows(html)![0]).toBe('lexicon');
    expect(rows(html)).not.toContain('movegen_margin');
    // A config read on its own has no colour: nothing is drawn in it.
    expect(html).not.toContain('player-color');
  });
});
