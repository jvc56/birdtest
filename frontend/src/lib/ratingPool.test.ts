import { describe, expect, it } from 'vitest';
import type { PlayerConfig, RatingHeadToHead, RatingPoolDetail, RatingRow } from '$lib/api';
import { cellTitle, crossTable, oneSide, poolMembership, recordSide, spreadText, winText } from './ratingPool';

function config(id: string): PlayerConfig {
  return { id, name: `config ${id}` } as PlayerConfig;
}

function rated(id: string, is_anchor = false): RatingRow {
  return {
    player_config_id: id,
    name: `config ${id}`,
    rating: 2000,
    stderr: 10,
    pairs_played: 4,
    connected_to_anchor: true,
    is_anchor
  };
}

function detail(members: string[], ratings: RatingRow[]): RatingPoolDetail {
  return {
    id: 'pool',
    name: 'pool',
    variant: 'classic',
    letter_distribution: 'english',
    layout: 'standard15',
    anchor_player_config_id: 'a',
    anchor_rating: 2000,
    members: members.map((id) => ({ player_config_id: id, name: `config ${id}` })),
    run: null,
    ratings,
    head_to_heads: []
  };
}

const configs = ['a', 'b', 'c', 'd'].map(config);

/** F-RATE-1: membership comes from the pool's members, not the latest fit. */
describe('poolMembership', () => {
  it('a never-fitted pool counts its anchor a member, not something to add', () => {
    const m = poolMembership(detail(['a'], []), configs);
    expect([...m.memberIds]).toEqual(['a']);
    expect(m.unrated.map((u) => u.player_config_id)).toEqual(['a']);
    expect(m.others.map((c) => c.id)).toEqual(['b', 'c', 'd']);
  });

  it('a member the latest fit has not rated is listed unrated and not offered to add', () => {
    const m = poolMembership(detail(['a', 'b', 'c'], [rated('a', true), rated('b')]), configs);
    expect(m.unrated.map((u) => u.player_config_id)).toEqual(['c']);
    expect(m.others.map((c) => c.id)).toEqual(['d']);
    expect(m.memberIds.has('c')).toBe(true);
  });

  it('a config removed since the latest fit is rated but no longer a member', () => {
    const m = poolMembership(detail(['a'], [rated('a', true), rated('b')]), configs);
    expect(m.memberIds.has('b')).toBe(false);
    expect(m.unrated).toEqual([]);
    expect(m.others.map((c) => c.id)).toEqual(['b', 'c', 'd']);
  });
});

function h2h(row: string, col: string, over: Partial<RatingHeadToHead> = {}): RatingHeadToHead {
  return { row, col, pairs: 20, actual: 0.5876, predicted: 0.56, stderr: 0.061934, spread: 6.8, ...over };
}

/** F-RATE-2: the cross table's order, lookup and cell text. */
describe('crossTable', () => {
  it('orders rated configs best first, then those with no chain to the anchor', () => {
    const row = (id: string, rating: number, connected = true): RatingRow => ({
      ...rated(id),
      rating,
      connected_to_anchor: connected
    });
    const table = crossTable({
      ratings: [row('a', 2000), row('island', 2600, false), row('b', 2150), row('c', 1890)],
      head_to_heads: [h2h('a', 'b'), h2h('b', 'a', { actual: 0.4124, spread: -6.8 })]
    });
    expect(table.configs.map((c) => c.player_config_id)).toEqual(['b', 'a', 'c', 'island']);
    expect(table.cell('a', 'b')?.actual).toBe(0.5876);
    expect(table.cell('b', 'a')?.spread).toBe(-6.8);
    expect(table.cell('a', 'c')).toBeUndefined();
  });

  it('counts each head-to-head once for the residual checks', () => {
    const cells = [h2h('a', 'b'), h2h('b', 'a'), h2h('b', 'c'), h2h('c', 'b')];
    expect(oneSide(cells).map((c) => c.row + c.col)).toEqual(['ab', 'bc']);
  });

  it('shows the win % with its standard error in percentage points, and the spread signed', () => {
    expect(winText(h2h('a', 'b'))).toBe('58.8% ±6.2');
    expect(winText(h2h('a', 'b', { stderr: 0 }))).toBe('58.8% ±0.0');
    expect(spreadText(h2h('a', 'b'))).toBe('+6.8');
    expect(spreadText(h2h('b', 'a', { spread: -12.46 }))).toBe('-12.5');
    // No "-0.0" for a spread that rounds to nothing.
    expect(spreadText(h2h('a', 'b', { spread: -0.04 }))).toBe('0.0');
  });

  it('sides a record by the figure the cell shows, not the float behind it', () => {
    expect(recordSide(h2h('a', 'b', { actual: 0.501 }))).toBe('win');
    expect(recordSide(h2h('a', 'b', { actual: 0.499 }))).toBe('loss');
    // Both show 50.0%: even, whatever they are to fifteen places.
    expect(recordSide(h2h('a', 'b', { actual: 0.5004 }))).toBe('even');
    expect(recordSide(h2h('a', 'b', { actual: 0.49951 }))).toBe('even');
    expect(recordSide(h2h('a', 'b', { actual: 0.5 }))).toBe('even');
  });

  it('spells the cell out in its hover, with what the ratings predict', () => {
    expect(cellTitle(h2h('a', 'b', { pairs: 1234 }), 'simmer', 'static')).toBe(
      'simmer against static: 58.8% ±6.2 over 1,234 pairs, average spread +6.8. ' +
        'The ratings predict 56.0% (residual +2.8 percentage points).'
    );
    expect(cellTitle(h2h('a', 'b', { pairs: 1 }), 'x', 'y')).toMatch(/ over 1 pair, /);
  });
});
