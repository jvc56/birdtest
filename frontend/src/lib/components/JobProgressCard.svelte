<script lang="ts">
  /**
   * A job's progress, on its public page and its admin page alike: how far
   * along it is over a bar, then four figures -- what is done, how fast it is
   * going now, the time left at that pace, and its share of claims -- and its
   * tasks by state in a line. Its status is a card of its own above it
   * (JobStatusCard), with the context it needs. Anything the page adds (who
   * created the job) goes in the slot at the foot.
   */
  import type { JobStats } from '$lib/api';
  import { jobProgress, throughputFigure, timeLeftFigure } from '$lib/jobProgress';

  export let stats: JobStats;

  $: progress = jobProgress(stats);
  // Read again with each update, so the clock time moves with the pace.
  $: figures = [
    { label: 'Done', ...progress.work, id: 'done' },
    { label: 'Throughput', ...throughputFigure(stats), id: 'throughput' },
    { label: 'Time left', ...timeLeftFigure(stats, Date.now()), id: 'time-left' },
    { label: 'Allocation', value: `${stats.job.allocation}%`, sub: 'of claims', id: 'allocation' }
  ];
  $: tasks = [
    { label: 'in progress', value: stats.tasks_claimed },
    { label: 'waiting to be reissued', value: stats.tasks_available },
    { label: 'done', value: stats.tasks_completed }
  ];
</script>

<div class="card space-y-4" data-testid="job-progress">
  <div class="flex items-baseline justify-between gap-3">
    <h2 class="text-lg font-medium">Progress</h2>
    <span class="tabular-nums {progress.done ? 'text-success' : ''}" data-testid="progress-headline">{progress.headline}</span>
  </div>
  <div
    class="h-3 overflow-hidden rounded-md bg-muted"
    role="progressbar"
    aria-label="Progress"
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(progress.fraction * 100)}
  >
    <div class="h-full transition-[width] {progress.done ? 'bg-success' : 'bg-primary'}" style="width: {progress.fraction * 100}%"></div>
  </div>
  {#if progress.stopped}
    <p class="-mt-2 text-xs text-muted-foreground" data-testid="progress-stopped">{progress.stopped}</p>
  {/if}
  <!-- Two across on a phone, four once there is room. -->
  <dl class="grid grid-cols-2 gap-3 sm:grid-cols-4" data-testid="progress-figures">
    {#each figures as figure}
      <div class="min-w-0 space-y-0.5" data-testid="progress-{figure.id}">
        <dt class="text-[11px] uppercase tracking-wider text-muted-foreground">{figure.label}</dt>
        <dd class="text-lg tabular-nums [overflow-wrap:anywhere]">{figure.value}</dd>
        <dd class="text-xs tabular-nums text-muted-foreground [overflow-wrap:anywhere]">{figure.sub}</dd>
      </div>
    {/each}
  </dl>
  <p class="text-[13px] tabular-nums text-muted-foreground" data-testid="task-counts">
    Tasks:{#each tasks as task, i}{i ? ' ·' : ''}
      <span class="text-foreground">{task.value.toLocaleString()}</span> {task.label}{/each}
  </p>
  <slot />
</div>
