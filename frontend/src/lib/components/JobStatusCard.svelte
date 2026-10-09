<script lang="ts">
  /**
   * Where a job stands, on a row of its own above the headline figures: the
   * status badge and, unless the job is active (which the badge says
   * plainly), what it means for this job: that its tasks are not being
   * offered, or, once finished, when and why it finished. The why used
   * to be a note above the figures, apart from the status it explained. Its
   * allocation is a card of its own, so this says nothing of it.
   */
  import type { JobStats } from '$lib/api';
  import { completionText, datetime, testLabel, testState } from '$lib/format';
  import JobStatusBadge from './JobStatusBadge.svelte';

  export let stats: JobStats;

  $: status = stats.job.status;
  $: test = stats.games?.test && status !== 'completed' ? testState(status, stats.games) : null;
</script>

<div class="card flex flex-wrap items-baseline gap-x-4 gap-y-1" data-testid="job-status">
  <p class="text-xs uppercase text-muted-foreground">Status</p>
  <p class="text-xl"><JobStatusBadge {status} /></p>
  {#if status !== 'active'}
    <p class="min-w-0 flex-1 basis-64 text-sm" data-testid="job-status-context">
      {#if status === 'completed'}
        <span class="font-medium">Finished</span>{stats.completion
          ? ` ${datetime(stats.completion.at)}`
          : ''}: {completionText(stats)}.
      {:else}
        Paused: no worker is offered its tasks until it is given an allocation.
      {/if}
      {#if test}
        <span class="text-muted-foreground">Its significance test is {testLabel(test)} (see the Significance Test card).</span>
      {/if}
    </p>
  {/if}
</div>
