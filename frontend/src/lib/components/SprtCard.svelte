<script lang="ts">
  /**
   * A games or pairs job's SPRT: where the test stands, the verdict it was
   * completed on, and for a pairs job the pentanomial it runs on. Nothing for
   * a job that runs no test. The job's record, which is not the test, is the
   * match score box above it.
   */
  import type { JobStats } from '$lib/api';
  import { sprtLabel, sprtState } from '$lib/format';
  import { pentanomialRows } from '$lib/charts/pentanomial';
  import JobStatusBadge from './JobStatusBadge.svelte';

  export let stats: JobStats;

  $: games = stats.games;
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
    {#if games.pentanomial}
      <div class="space-y-1">
        <p class="text-xs text-muted-foreground">
          The test runs on all {games.units_completed.toLocaleString()} pairs, scored by
          player 1's result across the pair. Pairs whose two games played identically are 1-1
          ties — they stay in the sample, where they are what makes a paired run
          lower-variance than an unpaired one.
        </p>
        <div class="overflow-x-auto">
        <table class="table text-xs">
          <thead>
            <tr>
              <th>Pair outcome</th>
              <th class="text-right">Pairs</th>
              <th class="text-right">Share</th>
            </tr>
          </thead>
          <tbody>
            {#each pentanomialRows(games.pentanomial, games.units_completed) as bucket}
              <tr>
                <td>{bucket.label}</td>
                <td class="text-right tabular-nums">{bucket.pairs.toLocaleString()}</td>
                <td class="text-right tabular-nums">{bucket.share}%</td>
              </tr>
            {/each}
          </tbody>
        </table>
        </div>
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
