<script lang="ts">
  /**
   * A job's tasks by state, saying what a task is and what each state means.
   * They count tasks -- batches handed to workers -- not games, pairs or
   * racks, and tasks are made on demand as workers ask, so none of them is
   * the work left.
   */
  import type { JobStats } from '$lib/api';
  import type { JobConfig } from '$lib/jobSettings';

  export let stats: JobStats;
  export let config: JobConfig | null;

  const plural = (n: number, unit: string) => `${n.toLocaleString()} ${unit}${n === 1 ? '' : 's'}`;

  $: batch = config?.games
    ? plural(config.games.per_batch, config.games.unit)
    : config?.opening_racks
      ? plural(config.opening_racks.racks_per_batch, 'rack')
      : config?.leave_generation
        ? plural(config.leave_generation.num_iterations, 'game')
        : null;

  $: counts = [
    {
      label: 'Waiting to be reissued',
      value: stats.tasks_available,
      help: "Made, then given back — its worker's claim lapsed, or it declined — and offered to the next worker before any new task is made."
    },
    { label: 'In progress', value: stats.tasks_claimed, help: 'Held by a worker now.' },
    { label: 'Done', value: stats.tasks_completed, help: 'Returned and accepted.' }
  ];
</script>

<div class="space-y-2" data-testid="task-counts">
  <h3 class="text-sm font-medium">Tasks</h3>
  <p class="text-xs text-muted-foreground">
    A task is one batch of work handed to a worker{#if batch}: {batch}{/if}. Tasks are made as
    workers ask for them, so these count what has been handed out so far, not the work left.
  </p>
  <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-3">
    {#each counts as count}
      <div>
        <dt class="text-muted-foreground">{count.label}</dt>
        <dd class="tabular-nums">{count.value.toLocaleString()}</dd>
        <dd class="text-xs text-muted-foreground">{count.help}</dd>
      </div>
    {/each}
  </dl>
</div>
