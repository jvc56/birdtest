import { describe, expect, it } from 'vitest';
import type { PlayerConfig, RatingPoolDetail, RatingRow } from '$lib/api';
import { poolMembership } from './ratingPool';

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
    residuals: []
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
