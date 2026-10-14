<script lang="ts">
  /**
   * A job's progress, on its public page and its admin page alike: how far
   * along it is over a bar, then four figures -- what is done, how fast it is
   * going now, the time left at that pace, and its share of claims -- and its
   * tasks by state in a line. An opening-rack job's bar has two layers, racks
   * settled over racks analysed, and a table gives each its Done and Time
   * left in place of those two figures; its tasks line splits the first pass
   * from re-analysis. Its status is a card of its own above it
   * (JobStatusCard), with the context it needs. Anything the page adds (who
   * created the job) goes in the slot at the foot.
   */
  import type { JobStats } from '$lib/api';
  import { jobProgress, rackProgress, taskCounts, throughputFigure, timeLeftFigure } from '$lib/jobProgress';

  export let stats: JobStats;

  $: progress = jobProgress(stats);
  // Read again with each update, so the clock times move with the pace.
  $: racks = rackProgress(stats, Date.now());
  $: figures = racks
    ? [
        { label: 'Throughput', ...throughputFigure(stats), id: 'throughput' },
        { label: 'Allocation', value: `${stats.job.allocation}%`, sub: 'of claims', id: 'allocation' }
      ]
    : [
        { label: 'Done', ...progress.work, id: 'done' },
        { label: 'Throughput', ...throughputFigure(stats), id: 'throughput' },
        { label: 'Time left', ...timeLeftFigure(stats, Date.now()), id: 'time-left' },
        { label: 'Allocation', value: `${stats.job.allocation}%`, sub: 'of claims', id: 'allocation' }
      ];
  $: tasks = taskCounts(stats);
  const pct = (f: number) => `${(f * 100).toFixed(1)}%`;
  const swatch = { analysed: 'bg-primary/35', settled: 'bg-primary' } as const;
</script>

<div class="card space-y-4" data-testid="job-progress">
  <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
    <h2 class="text-lg font-medium">Progress</h2>
    <span class="tabular-nums {progress.done ? 'text-success' : ''}" data-testid="progress-headline"
      >{racks ? racks.headline : progress.headline}</span
    >
  </div>
  {#if racks}
    <!-- Settled racks solid over racks analysed faint; the rest the track. -->
    <div
      class="relative h-3 overflow-hidden rounded-md bg-muted"
      role="progressbar"
      aria-label="Progress"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(racks.settled * 100)}
      aria-valuetext={racks.consensus
        ? `${pct(racks.settled)} settled, ${pct(racks.analysed)} analysed`
        : `${pct(racks.settled)} settled`}
    >
      {#if racks.consensus && !progress.done}
        <div class="absolute inset-y-0 left-0 bg-primary/35 transition-[width]" style="width: {racks.analysed * 100}%"></div>
      {/if}
      <div
        class="absolute inset-y-0 left-0 transition-[width] {progress.done ? 'bg-success' : 'bg-primary'}"
        style="width: {(progress.done ? 1 : racks.settled) * 100}%"
      ></div>
    </div>
  {:else}
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
  {/if}
  {#if progress.stopped}
    <p class="-mt-2 text-xs text-muted-foreground" data-testid="progress-stopped">{progress.stopped}</p>
  {/if}
  {#if racks}
    <div class="overflow-x-auto">
      <table class="w-full tabular-nums" data-testid="rack-fronts">
        <thead>
          <tr class="text-left text-[11px] uppercase tracking-wider text-muted-foreground">
            <th class="pb-1.5 pr-3 font-normal">Racks</th>
            <th class="pb-1.5 pr-3 font-normal">Done</th>
            <th class="pb-1.5 font-normal">Time left</th>
          </tr>
        </thead>
        <tbody>
          {#each racks.rows as row}
            <tr class="border-t border-border align-top" data-testid="rack-front-{row.key}">
              <td class="whitespace-nowrap py-2 pr-3 text-sm">
                <span class="mr-1.5 inline-block h-2.5 w-2.5 rounded-sm {swatch[row.key]}" aria-hidden="true"></span>{row.label}
              </td>
              <td class="py-2 pr-3">
                <span class="block text-lg [overflow-wrap:anywhere]">{row.done.toLocaleString()}</span>
                <span class="block text-xs text-muted-foreground">of {row.total.toLocaleString()} · {pct(row.fraction)}</span>
              </td>
              <td class="py-2">
                <span class="block text-lg">{row.left.value}</span>
                <span class="block text-xs text-muted-foreground [overflow-wrap:anywhere]">{row.left.sub}</span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
  <!-- Two across on a phone, four once there is room. -->
  <dl class="grid grid-cols-2 gap-3 {racks ? '' : 'sm:grid-cols-4'}" data-testid="progress-figures">
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
