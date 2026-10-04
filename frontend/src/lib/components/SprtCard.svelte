<script lang="ts">
  /**
   * A games or pairs job's SPRT: where the test stands, the verdict it was
   * completed on, what the LLR and its bounds mean with this job's own
   * settings, and for a pairs job the pair outcomes it runs on, a column per
   * player. Nothing for a job that runs no test. The job's record, which is
   * not the test, is the match score box above it.
   */
  import type { JobStats } from '$lib/api';
  import type { JobConfig } from '$lib/jobSettings';
  import { sprtLabel, sprtState } from '$lib/format';
  import { pentanomialRows } from '$lib/charts/pentanomial';
  import JobStatusBadge from './JobStatusBadge.svelte';
  import PlayerCompareTable from './PlayerCompareTable.svelte';

  export let stats: JobStats;
  /** The job's settings, for the hypotheses and error rates; the card explains without them. */
  export let config: JobConfig | null = null;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  $: games = stats.games;
  $: test = config?.games;
  $: names = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'] as [string, string];
  // Where the LLR sits between its bounds, for the bar: 0 at the lower, 1 at the upper.
  $: position = games?.sprt
    ? Math.min(1, Math.max(0, (games.sprt.llr - games.sprt.lower_bound) / (games.sprt.upper_bound - games.sprt.lower_bound)))
    : 0;
</script>

{#if games?.sprt}
  <div class="card space-y-4">
    <div class="flex items-center justify-between">
      <h2 class="text-lg font-medium">SPRT</h2>
      <JobStatusBadge status={sprtState(stats.job.status, games)} />
    </div>
    {#if games.decided}
      <p class="text-sm text-muted-foreground">
        Completed: {sprtLabel(games.decided.status)}, LLR
        {games.decided.llr.toFixed(3)} after {games.decided.units.toLocaleString()}
        {games.unit}{games.decided.units === 1 ? '' : 's'}. With the {games.unit}s that were in flight then, LLR
        {games.sprt.llr.toFixed(3)}, bounds [{games.sprt.lower_bound.toFixed(2)},
        {games.sprt.upper_bound.toFixed(2)}].
      </p>
    {:else if stats.job.status !== 'active'}
      <!-- Nothing is being played: the test is where it stopped, and said
           "running" as if it were not. -->
      <p class="text-sm text-muted-foreground">
        {sprtLabel(sprtState(stats.job.status, games))}{stats.job.status === 'inactive'
          ? `: no ${games.unit}s are being played, so the test is not moving`
          : ''}. LLR {games.sprt.llr.toFixed(3)}, bounds
        [{games.sprt.lower_bound.toFixed(2)}, {games.sprt.upper_bound.toFixed(2)}].
      </p>
    {:else}
      <p class="text-sm text-muted-foreground">
        {sprtLabel(games.sprt.status)} — LLR {games.sprt.llr.toFixed(3)}, bounds
        [{games.sprt.lower_bound.toFixed(2)}, {games.sprt.upper_bound.toFixed(2)}].
        {#if games.min_units > 0 && games.units_completed < games.min_units}
          SPRT is not acted on until {games.min_units.toLocaleString()}
          {games.unit}{games.min_units === 1 ? ' is' : 's are'} complete.
        {:else if games.min_units > 0}
          The minimum of {games.min_units.toLocaleString()}
          {games.unit}{games.min_units === 1 ? '' : 's'} is reached; SPRT is checked as
          {games.unit}s arrive.
        {:else}
          SPRT is checked as {games.unit}s arrive, with no minimum number of them.
        {/if}
      </p>
    {/if}
    <div class="space-y-1" aria-hidden="true">
      <div class="relative h-2 rounded-full bg-muted" data-testid="llr-bar">
        <div
          class="absolute top-1/2 h-4 w-1 -translate-x-1/2 -translate-y-1/2 rounded bg-foreground"
          style="left: {position * 100}%"
        ></div>
      </div>
      <div class="flex justify-between text-xs tabular-nums text-muted-foreground">
        <span>fails at {games.sprt.lower_bound.toFixed(2)}</span>
        <span>LLR {games.sprt.llr.toFixed(3)}</span>
        <span>passes at {games.sprt.upper_bound.toFixed(2)}</span>
      </div>
    </div>
    <details class="text-sm" data-testid="sprt-explained">
      <summary class="cursor-pointer text-muted-foreground">What do the LLR and bounds mean?</summary>
      <div class="mt-2 space-y-2 text-muted-foreground">
        <p>
          The test weighs two hypotheses about how much stronger player 1 is than player 2:
          {#if test}<strong>H0</strong>, by {test.elo_low} Elo, and <strong>H1</strong>, by
            {test.elo_high} Elo{:else}<strong>H0</strong>, the lower Elo difference it was set,
            and <strong>H1</strong>, the higher{/if}.
          {#if games.unit === 'pair'}
            Each completed pair is one observation, scored by player 1's result across its two
            games: 0, ½, 1, 1½ or 2 points. Pairing the games (the same racks, the seats
            swapped) cancels much of the luck of the draw, so a pairs job decides sooner than
            one playing as many separate games.
          {:else}
            Each game is one observation, scored 0, ½ or 1 for player 1.
          {/if}
        </p>
        <p>
          The <strong>LLR</strong> (log-likelihood ratio) is the evidence so far: how much more
          likely the results are under H1 than under H0, as a logarithm. It rises as player 1
          does better than H0 expects, and falls as it does worse than H1 expects.
        </p>
        <p>
          The test <strong>passes</strong> (accepts H1) once the LLR reaches the upper bound,
          {games.sprt.upper_bound.toFixed(2)} = ln((1 − β) / α), and <strong>fails</strong>
          (accepts H0) once it reaches the lower bound, {games.sprt.lower_bound.toFixed(2)} =
          ln(β / (1 − α)){#if test}, where α = {test.sprt_alpha} is the chance of passing when
            H0 is true and β = {test.sprt_beta} the chance of failing when H1 is{:else}, where α
            and β are the error rates it accepts{/if}. Until then it keeps playing, up to its cap.
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
