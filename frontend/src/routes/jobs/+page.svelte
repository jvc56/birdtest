<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type JobListItem, type Page } from '$lib/api';
  import { jobTypeLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';
  import Pagination from '$lib/components/Pagination.svelte';

  let result: Page<JobListItem> | null = null;
  let page = 0;

  async function load(next: number) {
    page = next;
    result = await api.jobs(next);
  }

  onMount(() => load(0));

  /** On-demand SPRT jobs count games or pairs; everything else counts tasks. */
  function progress(job: JobListItem): { value: number; max: number; unit: string } {
    if (job.units_completed !== null && job.max_units !== null) {
      return { value: job.units_completed, max: job.max_units, unit: 'units' };
    }
    return { value: job.tasks_completed, max: job.tasks_total, unit: 'tasks' };
  }
</script>

<h1 class="mb-6 text-2xl font-semibold">Jobs</h1>

{#if !result}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="card overflow-x-auto p-0">
    <table class="table">
      <thead>
        <tr>
          <th>Type</th><th>Status</th><th>Priority</th><th>Allocation</th>
          <th>Redundancy</th><th class="text-right">Progress</th>
        </tr>
      </thead>
      <tbody>
        {#each result.items as job}
          {@const p = progress(job)}
          <tr>
            <td><a href="/jobs/{job.id}">{jobTypeLabel(job.job_type)}</a></td>
            <td><JobStatusBadge status={job.status} /></td>
            <td class="tabular-nums">{job.priority}</td>
            <td class="tabular-nums">{job.allocation === null ? '—' : `${job.allocation}%`}</td>
            <td class="tabular-nums">{job.redundancy}×</td>
            <td class="text-right tabular-nums">
              {p.value.toLocaleString()} / {p.max.toLocaleString()}
              <span class="text-muted-foreground">{p.unit}</span>
            </td>
          </tr>
        {:else}
          <tr><td colspan="6" class="text-muted-foreground">No jobs yet.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
  <Pagination page={result.page} perPage={result.per_page} total={result.total} onChange={load} />
{/if}
