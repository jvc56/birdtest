<script lang="ts">
  import type { JobStats } from '$lib/api';
  import { duration } from '$lib/format';

  /**
   * A job's four headline figures, on its public page and its admin page
   * alike: its share of claims, how much is done, how many contributors are
   * working on it right now, and how long the rest should take. Its status is
   * a row of its own above them (JobStatusCard), with the context it needs.
   * The job's movegens are under its Contributors table.
   */
  export let stats: JobStats;
</script>

<!-- The page's only grid of cards: E-10 counts `.grid > .card` for these four.
     Two across until there is room for four. -->
<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
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
    <p class="text-xs uppercase text-muted-foreground">Active contributors</p>
    <p class="mt-1 text-xl tabular-nums">{stats.active_contributors.toLocaleString()}</p>
  </div>
  <div class="card">
    <p class="text-xs uppercase text-muted-foreground">Estimated time left</p>
    <p class="mt-1 text-xl tabular-nums">{duration(stats.eta_seconds)}</p>
  </div>
</div>
