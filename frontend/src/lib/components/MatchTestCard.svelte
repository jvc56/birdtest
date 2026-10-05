<script lang="ts">
  /**
   * A games or pairs job's significance test: where it stands, the verdict it was
   * completed on, player 1's score interval on a bar around an even score,
   * what the interval means with this job's own confidence, and for a pairs
   * job the pair outcomes it runs on, a column per player. Nothing for a job
   * that runs no test. The job's record, which is not the test, is the match
   * score box above it.
   */
  import type { JobStats } from '$lib/api';
  import { scorePct, testLabel, testState } from '$lib/format';
  import { testBar, testSentence } from '$lib/matchTest';
  import { pentanomialRows } from '$lib/charts/pentanomial';
  import JobStatusBadge from './JobStatusBadge.svelte';
  import PlayerCompareTable from './PlayerCompareTable.svelte';

  export let stats: JobStats;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  $: games = stats.games;
  $: names = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'] as [string, string];
  $: bar = games?.test ? testBar(games.test) : null;
</script>

{#if games?.test && bar}
  <div class="card space-y-4" data-testid="significance-test">
    <div class="flex items-center justify-between">
      <h2 class="text-lg font-medium">Significance Test</h2>
      <JobStatusBadge status={testState(stats.job.status, games)} />
    </div>
    <p class="text-sm" data-testid="significance-test-sentence">{testSentence(games.test, names)}</p>
    {#if games.decided}
      <p class="text-sm text-muted-foreground">
        Completed: {testLabel(games.decided.status)}, player 1 at {scorePct(games.decided.lower)} to
        {scorePct(games.decided.upper)} after {games.decided.units.toLocaleString()}
        {games.unit}{games.decided.units === 1 ? '' : 's'}. The figures above include the
        {games.unit}s that were in flight then.
      </p>
    {:else if stats.job.status !== 'active'}
      <!-- Nothing is being played: the test is where it stopped, and said
           "running" as if it were not. -->
      <p class="text-sm text-muted-foreground">
        {testLabel(testState(stats.job.status, games))}{stats.job.status === 'inactive'
          ? `: no ${games.unit}s are being played, so the test is not moving`
          : ''}.
      </p>
    {:else if games.units_completed < games.min_units}
      <p class="text-sm text-muted-foreground">
        The test is not acted on until {games.min_units.toLocaleString()}
        {games.unit}{games.min_units === 1 ? ' is' : 's are'} complete.
      </p>
    {:else}
      <p class="text-sm text-muted-foreground">
        The minimum of {games.min_units.toLocaleString()}
        {games.unit}{games.min_units === 1 ? '' : 's'} is reached; the test is checked as
        {games.unit}s arrive, and the job stops when it decides or at
        {games.max_units.toLocaleString()}.
      </p>
    {/if}
    <div class="space-y-1" aria-hidden="true">
      <div class="relative h-3 rounded-full bg-muted" data-testid="test-bar">
        <div
          class="absolute top-0 h-3 rounded-full bg-primary/40"
          style="left: {bar.lower}%; width: {Math.max(bar.upper - bar.lower, 0.5)}%"
        ></div>
        <div
          class="absolute top-1/2 h-5 w-px -translate-y-1/2 border-l border-dashed border-foreground/60"
          style="left: {bar.even}%"
        ></div>
        <div
          class="absolute top-1/2 h-4 w-1 -translate-x-1/2 -translate-y-1/2 rounded bg-foreground"
          style="left: {bar.mean}%"
        ></div>
      </div>
      <div class="flex justify-between text-xs tabular-nums text-muted-foreground">
        <span>{scorePct(bar.from)}</span>
        <span>even at 50%, {names[0]}'s score {scorePct(games.test.mean)}</span>
        <span>{scorePct(bar.to)}</span>
      </div>
    </div>
    <details class="text-sm" data-testid="test-explained">
      <summary class="cursor-pointer text-muted-foreground">What does the interval mean?</summary>
      <div class="mt-2 space-y-2 text-muted-foreground">
        <p>
          The test asks whether one player is better than the other, at {games.test.confidence_pct}%
          confidence. It keeps an interval around player 1's score per game (1 for a win, ½ for a
          draw):
          {#if games.unit === 'pair'}
            each completed pair is one observation, scored by player 1's result across its two
            games. Pairing the games (the same racks, the seats swapped) cancels much of the luck
            of the draw, so a pairs job decides sooner than one playing as many separate games.
          {:else}
            each game is one observation.
          {/if}
        </p>
        <p>
          Once the interval lies wholly above 50%, player 1 is better; wholly below, player 2 is.
          Unlike an ordinary confidence interval it stays valid however often it is checked, so
          the job can stop the moment it decides: the chance of naming a winner between two equal
          players is at most about {Math.round((100 - games.test.confidence_pct) * 10) / 10}%. If
          it has not decided by the job's cap, neither player is shown to be better, and the
          interval says how large a difference the games rule out.
        </p>
      </div>
    </details>
    {#if games.pentanomial}
      <div class="space-y-1">
        <p class="text-xs text-muted-foreground">
          The test runs on all {games.units_completed.toLocaleString()} pairs, each scored by
          its result across the pair, from each player's side. Pairs whose two games played
          identically are even — they stay in the sample, where they are what makes a paired
          run lower-variance than an unpaired one.
        </p>
        <PlayerCompareTable
          players={names}
          caption="Pair outcome"
          rows={pentanomialRows(games.pentanomial, games.units_completed)}
        />
        {#if games.divergent_pairs !== undefined}
          <p class="text-xs text-muted-foreground">
            {games.divergent_pairs.toLocaleString()} of {games.units_completed.toLocaleString()}
            pairs diverged — a diagnostic of how often these two configs differ at all, not
            part of the test.
          </p>
        {/if}
      </div>
    {/if}
  </div>
{/if}
