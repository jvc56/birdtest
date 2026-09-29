<script lang="ts">
  import type { JobStats } from '$lib/api';
  import { duration } from '$lib/format';
  import JobStatusBadge from './JobStatusBadge.svelte';

  /**
   * A job's four headline figures, on its public page and its admin page
   * alike: where it stands, its share of claims, how much is done, and how
   * long the rest should take. The status was a badge in the title, where it
   * read as part of the name; redundancy, which rarely differs between jobs,
   * is under the settings' "All settings".
   */
  export let stats: JobStats;
</script>

<!-- The page's only grid of cards: E-10 counts `.grid > .card` for these four. -->
<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
  <div class="card" data-testid="job-status">
    <p class="text-xs uppercase text-muted-foreground">Status</p>
    <p class="mt-1 text-xl"><JobStatusBadge status={stats.job.status} /></p>
  </div>
  <div class="card">
    <p class="text-xs uppercase text-muted-foreground">Allocation</p>
    <p class="mt-1 text-xl tabular-nums">
      {stats.job.allocation === null ? '—' : `${stats.job.allocation}%`}
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
