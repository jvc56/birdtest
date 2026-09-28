<script lang="ts">
  /**
   * Why a completed job finished, where it is seen first: under the job's
   * title. A pairs job stopped by its test and one stopped at its cap both
   * read "completed", and the difference was a line deep in the SPRT card.
   */
  import type { JobStats } from '$lib/api';
  import { completionText, datetime } from '$lib/format';

  export let stats: JobStats;
</script>

{#if stats.job.status === 'completed'}
  <p class="rounded-md border border-primary/30 bg-primary/10 px-3 py-2 text-sm">
    <span class="font-medium">Finished</span>{stats.completion
      ? ` ${datetime(stats.completion.at)}`
      : ''}: {completionText(stats)}.
  </p>
{/if}
