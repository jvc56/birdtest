<script lang="ts">
  /**
   * A games or pairs job's result as a match: player 1's record and score,
   * the players' average scores and the spread, and the win/loss/draw chart.
   * On the public and admin job pages alike, after the settings and before
   * the SPRT card -- for a job that runs no test, this is its result.
   */
  import type { GameStats } from '$lib/api';
  import { matchScore } from '$lib/matchScore';
  import OutcomeChart from './OutcomeChart.svelte';

  export let games: GameStats;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  $: m = matchScore(games);
  $: [p1, p2] = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'];
</script>

<!-- A card, but not in a grid: E-10 counts `.grid > .card` for the stats row. -->
<div class="card space-y-4">
  <h2 class="text-lg font-medium">Match score</h2>
  <dl class="grid grid-cols-2 gap-x-6 gap-y-3 text-sm sm:grid-cols-4">
    <div>
      <dt class="text-muted-foreground">Player 1 W–L–D</dt>
      <dd class="text-xl tabular-nums" data-testid="match-record">{m.record}</dd>
    </div>
    <div>
      <dt class="text-muted-foreground">Score</dt>
      <dd class="text-xl tabular-nums">
        {m.score}
        <span class="text-sm text-muted-foreground">/ {m.games.toLocaleString()}</span>
      </dd>
    </div>
    <div>
      <dt class="text-muted-foreground" title="Player 1's score over the games played: a draw counts half">
        Win %
      </dt>
      <dd class="text-xl tabular-nums">{m.scorePct ?? '—'}</dd>
    </div>
    <div>
      <dt class="text-muted-foreground">Average spread</dt>
      <dd class="text-xl tabular-nums">{m.spread ?? '—'}</dd>
    </div>
  </dl>
  <p class="text-sm">
    <span class="text-muted-foreground">Average score:</span>
    <span class="break-all">{p1}</span>
    <span class="tabular-nums">{m.p1Mean ?? '—'}</span>
    ·
    <span class="break-all">{p2}</span>
    <span class="tabular-nums">{m.p2Mean ?? '—'}</span>
  </p>
  <OutcomeChart wins={games.wins} losses={games.losses} draws={games.draws} />
  <p class="text-sm tabular-nums text-muted-foreground">
    Player 1: {games.wins.toLocaleString()} W ({games.win_pct.toFixed(1)}%) ·
    {games.losses.toLocaleString()} L ({games.loss_pct.toFixed(1)}%) ·
    {games.draws.toLocaleString()} D ({games.draw_pct.toFixed(1)}%){#if games.unit === 'pair'}
      — per game, over both games of every pair{/if}
  </p>
</div>
