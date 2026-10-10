<script lang="ts">
  /**
   * Every job that can run, with its allocation, set together: the active
   * jobs may allocate at most 100% between them, checked as the page will
   * leave them rather than one job at a time -- moving 20% from one job to
   * another no longer means lowering the first before raising the second.
   * Above 0% a job is active (activated if it was not); at 0% it is inactive
   * (deactivated if it was active). Completed jobs are not listed: they cannot
   * run again.
   */
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { api, errorText, type JobListItem, type JobStatus } from '$lib/api';
  import { equalShares } from '$lib/allocation';
  import { jobTitle, jobTypeLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';

  let jobs: JobListItem[] = [];
  // The jobs the creation form just made (`?new=id,id`): a round robin's
  // pairings, inactive at 0%, marked so they can be found among the rest and
  // listed first.
  $: fresh = new Set(($page.url.searchParams.get('new') ?? '').split(',').filter(Boolean));
  $: listed = [...jobs].sort((a, b) => Number(fresh.has(b.id)) - Number(fresh.has(a.id)));
  /** The allocation each job is set to on the page, by id. */
  let values: Record<string, number> = {};
  let loaded = false;
  let busy = false;
  let error = '';
  let saved = '';

  async function every(status: JobStatus): Promise<JobListItem[]> {
    const all: JobListItem[] = [];
    for (let page = 0; ; page++) {
      const result = await api.jobs(page, status);
      all.push(...result.items);
      if (all.length >= result.total || result.items.length === 0) return all;
    }
  }

  async function load() {
    error = '';
    try {
      const [active, inactive] = await Promise.all([every('active'), every('inactive')]);
      jobs = [...active, ...inactive];
      values = Object.fromEntries(
        jobs.map((job) => [job.id, job.allocation])
      );
      loaded = true;
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(load);

  const current = (job: JobListItem) => job.allocation;
  $: total = jobs.reduce((sum, job) => sum + (Number(values[job.id]) || 0), 0);
  $: invalid = jobs.some((job) => {
    const v = values[job.id];
    return !Number.isInteger(v) || v < 0 || v > 100;
  });
  $: changed = jobs.filter((job) => Number(values[job.id]) !== current(job));

  /** The running and new jobs' share, the same whole number each. */
  function shareEqually() {
    values = equalShares(listed.map((job) => job.id), values, fresh);
  }

  async function save() {
    error = '';
    saved = '';
    busy = true;
    try {
      await api.setAllocations(
        changed.map((job) => ({ job_id: job.id, allocation: Number(values[job.id]) }))
      );
      saved = `Saved: ${changed.length} job${changed.length === 1 ? '' : 's'} changed.`;
      await load();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="space-y-4">
  <div>
    <h1 class="text-2xl font-semibold">Allocation</h1>
    <p class="text-sm text-muted-foreground">
      Each job's share of the claims workers make. The active jobs may allocate at most 100%
      between them; set them all here and save once. This is the only place a job is switched on
      or off: above 0% it is active, and at 0% it is inactive. Completed jobs cannot run again and
      are not listed.
    </p>
  </div>

  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  {#if saved}<p class="text-sm text-success" role="status">{saved}</p>{/if}

  {#if !loaded && !error}
    <p class="text-muted-foreground">Loading…</p>
  {:else if loaded && !jobs.length}
    <p class="text-muted-foreground">No job is active or inactive: there is nothing to allocate.</p>
  {:else if loaded}
    {#if fresh.size}
      <p class="text-sm" data-testid="new-jobs">
        {fresh.size === 1 ? 'The new job is' : `The ${fresh.size} new jobs are`} marked
        <span class="rounded-full border border-primary/30 bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">new</span> and listed first, inactive at 0%: give
        {fresh.size === 1 ? 'it an allocation' : 'them allocations'} to start
        {fresh.size === 1 ? 'it' : 'them'}.
      </p>
    {/if}
    <form class="card space-y-4" on:submit|preventDefault={save}>
      <div class="overflow-x-auto">
        <table class="table text-sm" data-testid="allocations">
          <thead>
            <tr>
              <th>Job</th><th>Type</th><th>Status</th><th class="text-right">Now</th>
              <th class="text-right">Allocation %</th>
            </tr>
          </thead>
          <tbody>
            {#each listed as job (job.id)}
              <tr class:font-medium={Number(values[job.id]) !== current(job)}>
                <td>
                  <a href="/admin/jobs/{job.id}">{jobTitle(job)}</a>
                  {#if fresh.has(job.id)}<span class="ml-1 rounded-full border border-primary/30 bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">new</span>{/if}
                </td>
                <td>{jobTypeLabel(job.job_type)}</td>
                <td><JobStatusBadge status={job.status} /></td>
                <td class="text-right tabular-nums">{current(job)}%</td>
                <td class="text-right">
                  <label class="sr-only" for="alloc-{job.id}">Allocation for {jobTitle(job)}</label>
                  <input
                    id="alloc-{job.id}"
                    type="number"
                    min="0"
                    max="100"
                    step="1"
                    class="input w-24 text-right"
                    bind:value={values[job.id]}
                  />
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="flex flex-wrap items-center gap-3">
        <p class="text-sm tabular-nums" data-testid="allocation-total">
          <span class:text-destructive={total > 100} class:font-semibold={total > 100}>{total}%</span>
          of 100% allocated{#if total > 100}: lower some jobs before saving{/if}
        </p>
        <button type="button" class="btn-secondary" on:click={shareEqually}>Share equally</button>
        <button
          type="submit"
          class="btn-primary ml-auto"
          disabled={busy || invalid || total > 100 || !changed.length}
        >
          {busy ? 'Saving…' : changed.length ? `Save ${changed.length} change${changed.length === 1 ? '' : 's'}` : 'No changes'}
        </button>
      </div>
      {#if invalid}
        <p class="field-error">Every allocation must be a whole number from 0 to 100.</p>
      {/if}
      <p class="text-xs text-muted-foreground">
        "Share equally" splits 100% among the jobs set above 0% and the new ones (all of them
        when there are none).
        Jobs you leave unchanged are not sent.
      </p>
    </form>
  {/if}
</div>
