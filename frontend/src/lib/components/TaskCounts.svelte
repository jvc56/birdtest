<script lang="ts">
  /**
   * A job's tasks by state: batches handed to workers, not games, pairs or
   * racks.
   */
  import type { JobStats } from '$lib/api';

  export let stats: JobStats;

  $: counts = [
    { label: 'Waiting to be reissued', value: stats.tasks_available },
    { label: 'In progress', value: stats.tasks_claimed },
    { label: 'Done', value: stats.tasks_completed }
  ];
</script>

<div class="space-y-2" data-testid="task-counts">
  <h3 class="text-sm font-medium">Tasks</h3>
  <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-3">
    {#each counts as count}
      <div>
        <dt class="text-muted-foreground">{count.label}</dt>
        <dd class="tabular-nums">{count.value.toLocaleString()}</dd>
      </div>
    {/each}
  </dl>
</div>
