<script lang="ts">
  import type { JobStats } from '$lib/api';
  import { duration, exactCount } from '$lib/format';

  /**
   * A job's four headline figures, on its public page and its admin page
   * alike: its share of claims, how much is done, the work that took -- its
   * movegens, to the last digit as the Contributors page counts them -- and
   * how long the rest should take. Its status is a row of its own above them
   * (JobStatusCard), with the context it needs.
   */
  export let stats: JobStats;
</script>

<!-- The page's only grid of cards: E-10 counts `.grid > .card` for these four.
     Two across until there is room for four: a job's movegens to the last
     digit is fourteen characters at a billion, and a quarter of a tablet's
     width was not enough for it. -->
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
    <p class="text-xs uppercase text-muted-foreground">Movegens</p>
    <p class="mt-1 break-all text-xl tabular-nums">{exactCount(stats.movegens)}</p>
  </div>
  <div class="card">
    <p class="text-xs uppercase text-muted-foreground">Estimated time left</p>
    <p class="mt-1 text-xl tabular-nums">{duration(stats.eta_seconds)}</p>
  </div>
</div>
