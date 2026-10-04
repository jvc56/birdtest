<script lang="ts">
  /**
   * A games or pairs job's result as a match: a column per player, the higher
   * figure of each row green and the lower red. On the public and admin job
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
</script>

<!-- A card, but not in a grid: E-10 counts `.grid > .card` for the stats row. -->
<div class="card space-y-3">
  <h2 class="text-lg font-medium">Match score</h2>
  <PlayerCompareTable players={names} rows={matchRows(games)} />
  <p class="text-xs text-muted-foreground" data-testid="match-games">
    Over {played.toLocaleString()} game{played === 1 ? '' : 's'}{#if games.unit === 'pair'}, both
      games of every pair{/if}. The higher figure in each row is green, the lower red.
  </p>
</div>
