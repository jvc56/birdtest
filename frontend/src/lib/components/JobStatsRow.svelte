<script lang="ts">
  import type { JobStats } from '$lib/api';
  import { duration } from '$lib/format';

  /**
   * A job's three headline figures, on its public page and its admin page
   * alike: its share of claims, how much is done, and how long the rest should
   * take. Its status is a row of its own above them (JobStatusCard), with the
   * context it needs.
   */
  export let stats: JobStats;
</script>

<!-- The page's only grid of cards: E-10 counts `.grid > .card` for these three. -->
<div class="grid gap-4 sm:grid-cols-3">
  <div class="card">
    <p class="text-xs uppercase text-muted-foreground">Allocation</p>
    <p class="mt-1 text-xl tabular-nums">
      {stats.job.allocation}%
    </p>
  </div>
  <div class="card">
    <p class="text-xs uppercase text-muted-foreground">Tasks completed</p>
    <p class="mt-1 text-xl tabular-nums">{stats.tasks_completed.toLocaleString()}</p>
  </div>
  <div class="card">
    <p class="text-xs uppercase text-muted-foreground">Estimated time left</p>
    <p class="mt-1 text-xl tabular-nums">{duration(stats.eta_seconds)}</p>
  </div>
</div>
