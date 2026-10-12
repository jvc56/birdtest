<script lang="ts">
  /**
   * A games or pairs job's result as a match: each player's share of the
   * points, a bar of the games each won with the draws between them, and each
   * player's average score and the spread. A pairs job's card switches between
   * all its games and only the games of the pairs that diverged -- where the
   * two configs actually played differently. On the public and admin job pages
   * alike, after the settings and before the Significance Test card -- for a
   * job that runs no test, this is its result.
   */
  import type { GameStats } from '$lib/api';
  import { matchScore } from '$lib/matchScore';

  export let games: GameStats;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  let scope: 'all' | 'divergent' = 'all';

  $: names = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'] as [string, string];
  $: divergent = games.divergent;
  $: all = matchScore(games);
  $: diverged = divergent ? matchScore(divergent) : null;
  $: shown = scope === 'divergent' && diverged ? diverged : all;
  const plural = (n: number, unit: string) => `${n.toLocaleString()} ${unit}${n === 1 ? '' : 's'}`;
  const text = ['text-player-1', 'text-player-2'] as const;
</script>

<!-- A card, but not in a grid: E-10 counts `.grid > .card` for the stats row. -->
<div class="card space-y-4" data-testid="match-score">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <h2 class="text-lg font-medium">Match score</h2>
    {#if diverged}
      <div class="inline-flex overflow-hidden rounded-md border border-border text-xs tabular-nums" role="group" aria-label="Which games">
        {#each [['all', 'All games', all.played], ['divergent', 'Diverged', diverged.played]] as [value, label, n], i}
          <button
            type="button"
            class="px-2.5 py-1 {i ? 'border-l border-border' : ''} {scope === value
              ? 'bg-muted text-foreground'
              : 'text-muted-foreground hover:text-foreground'}"
            aria-pressed={scope === value}
            data-testid="match-scope-{value}"
            on:click={() => (scope = value === 'divergent' ? 'divergent' : 'all')}>{label} · {n.toLocaleString()}</button
          >
        {/each}
      </div>
    {/if}
  </div>

  {#if shown.shares === null}
    <p class="text-sm text-muted-foreground" data-testid="match-games">
      {scope === 'divergent' ? 'No pair has diverged yet.' : 'No games played yet.'}
    </p>
  {:else}
    <div class="grid grid-cols-2 gap-3">
      {#each names as name, i}
        <div class="min-w-0 space-y-0.5 {i ? 'text-right' : ''}" data-testid="match-player-{i + 1}">
          <div class="text-sm font-medium [overflow-wrap:anywhere] {text[i]}">{name}</div>
          <div
            class="text-3xl tabular-nums {shown.leader === i + 1 ? 'font-semibold text-foreground' : 'text-muted-foreground'}"
            data-testid="match-share"
          >
            {shown.shares[i]}
          </div>
          <div class="text-xs text-muted-foreground">of points</div>
        </div>
      {/each}
    </div>

    <div class="space-y-1.5">
      <!-- Each part grows with its count from nothing, but never narrower than
           its number: a lopsided result still shows the smaller count. -->
      <div
        class="flex h-7 gap-0.5 overflow-hidden rounded-md text-sm font-semibold tabular-nums text-primary-foreground"
        role="img"
        aria-label="{names[0]} won {shown.wins[0].toLocaleString()}, {plural(shown.draws, 'draw')}, {names[1]} won {shown.wins[1].toLocaleString()}"
        data-testid="match-bar"
      >
        {#if shown.wins[0]}
          <span class="flex items-center whitespace-nowrap bg-player-1 px-2" style="flex: {shown.wins[0]} 1 0" data-testid="match-wins-1"
            >{shown.wins[0].toLocaleString()}</span
          >
        {/if}
        {#if shown.draws}
          <span class="min-w-[3px] bg-muted-foreground/50" style="flex: {shown.draws} 1 0"></span>
        {/if}
        {#if shown.wins[1]}
          <span
            class="flex items-center justify-end whitespace-nowrap bg-player-2 px-2"
            style="flex: {shown.wins[1]} 1 0"
            data-testid="match-wins-2">{shown.wins[1].toLocaleString()}</span
          >
        {/if}
      </div>
      <div class="flex justify-between gap-2 text-xs text-muted-foreground" aria-hidden="true">
        <span class="[overflow-wrap:anywhere]">{names[0]} wins</span>
        <span class="shrink-0 tabular-nums" data-testid="match-draws">{plural(shown.draws, 'draw')}</span>
        <span class="text-right [overflow-wrap:anywhere]">{names[1]} wins</span>
      </div>
    </div>

    <dl class="grid gap-x-6 gap-y-2 sm:grid-cols-2">
      <div>
        <dt class="text-xs text-muted-foreground" title="Points per game">Average score</dt>
        <dd class="tabular-nums" data-testid="match-averages">
          <span class={text[0]}>{shown.averages[0]}</span> to <span class={text[1]}>{shown.averages[1]}</span>
        </dd>
      </div>
      <div>
        <dt class="text-xs text-muted-foreground" title="Points ahead of the opponent at the end of a game, on average">
          Average spread per game
        </dt>
        <dd class="tabular-nums" data-testid="match-spread">
          {#if !shown.spread}—{:else if shown.spread.ahead === 0}Even{:else}+{shown.spread.points} to
            <span class={text[shown.spread.ahead - 1]}>{names[shown.spread.ahead - 1]}</span>{/if}
        </dd>
      </div>
    </dl>

    <p class="text-xs text-muted-foreground" data-testid="match-games">
      Over {plural(shown.played, 'game')}{#if scope === 'divergent'}: both games of the {plural(shown.played / 2, 'pair')}
        whose games did not play identically{:else if games.unit === 'pair'}, both games of every pair{/if}. A draw
      counts as half a point.
    </p>
  {/if}
</div>
