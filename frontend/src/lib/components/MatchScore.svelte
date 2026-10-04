<script lang="ts">
  /**
   * A games or pairs job's result as a match: a column per player, the better
   * figure of each row green and the worse red. A pairs job has a second
   * table beside it, over only the games of the pairs that diverged -- where
   * the two configs actually played differently. On the public and admin job
   * pages alike, after the settings and before the SPRT card -- for a job that
   * runs no test, this is its result.
   */
  import type { GameStats } from '$lib/api';
  import { gamesPlayed, matchRows } from '$lib/matchScore';
  import PlayerCompareTable from './PlayerCompareTable.svelte';

  export let games: GameStats;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  $: names = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'] as [string, string];
  $: played = gamesPlayed(games);
  $: divergent = games.divergent;
  $: divergentPlayed = divergent ? gamesPlayed(divergent) : 0;
  const plural = (n: number, unit: string) => `${n.toLocaleString()} ${unit}${n === 1 ? '' : 's'}`;
</script>

<!-- A card, but not in a grid: E-10 counts `.grid > .card` for the stats row. -->
<div class="card space-y-3">
  <h2 class="text-lg font-medium">Match score</h2>
  <div class={divergent ? 'grid gap-4 lg:grid-cols-2' : ''}>
    <div class="space-y-2" data-testid="match-all">
      {#if divergent}<h3 class="text-sm font-medium">All games</h3>{/if}
      <PlayerCompareTable players={names} rows={matchRows(games)} />
      <p class="text-xs text-muted-foreground" data-testid="match-games">
        Over {plural(played, 'game')}{#if games.unit === 'pair'}, both games of every pair{/if}.
      </p>
    </div>
    {#if divergent}
      <div class="space-y-2" data-testid="match-divergent">
        <h3 class="text-sm font-medium">Games that diverged</h3>
        <PlayerCompareTable players={names} rows={matchRows(divergent)} />
        <p class="text-xs text-muted-foreground">
          Over {plural(divergentPlayed, 'game')}: both games of the {plural(divergentPlayed / 2, 'pair')}
          whose games did not play identically. A pair played identically comes out even, so leaving
          those out shows where the players actually differ.
        </p>
      </div>
    {/if}
  </div>
  <p class="text-xs text-muted-foreground">The better figure in each row is green, the worse red.</p>
</div>
