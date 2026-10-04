<script lang="ts">
  /**
   * Where a job stands, on a row of its own above the headline figures: the
   * status badge and what it means for this job -- who is being offered its
   * tasks, or, once finished, when and why it finished. The why used to be a
   * note above the figures, apart from the status it explained.
   */
  import type { JobStats } from '$lib/api';
  import { completionText, datetime, sprtLabel, sprtState } from '$lib/format';
  import JobStatusBadge from './JobStatusBadge.svelte';

  export let stats: JobStats;

  $: status = stats.job.status;
  $: allocation = stats.job.allocation;
  $: test = stats.games?.sprt && status !== 'completed' ? sprtState(status, stats.games) : null;
</script>

<div class="card flex flex-wrap items-baseline gap-x-4 gap-y-1" data-testid="job-status">
  <p class="text-xs uppercase text-muted-foreground">Status</p>
  <p class="text-xl"><JobStatusBadge {status} /></p>
  <p class="min-w-0 flex-1 basis-64 text-sm" data-testid="job-status-context">
    {#if status === 'completed'}
      <span class="font-medium">Finished</span>{stats.completion
        ? ` ${datetime(stats.completion.at)}`
        : ''}: {completionText(stats)}.
    {:else if status === 'active'}
      {#if allocation}
        Workers are being offered its tasks: it gets {allocation}% of the claims the active jobs
        share.
      {:else}
        Active at 0%: no worker is offered its tasks until its allocation is raised.
      {/if}
    {:else}
      Paused: no worker is offered its tasks{allocation === null
        ? '. It has not been activated yet'
        : ` until it is activated again (it was at ${allocation}%)`}.
    {/if}
    {#if test}
      <span class="text-muted-foreground">Its SPRT is {sprtLabel(test)} (see the SPRT card).</span>
    {/if}
  </p>
</div>
