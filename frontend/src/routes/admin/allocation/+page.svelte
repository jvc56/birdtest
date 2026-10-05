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
  import { api, errorText, type JobListItem, type JobStatus } from '$lib/api';
  import { jobTitle, jobTypeLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';

  let jobs: JobListItem[] = [];
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
        jobs.map((job) => [job.id, job.status === 'active' ? (job.allocation ?? 0) : 0])
      );
      loaded = true;
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(load);

  const current = (job: JobListItem) => (job.status === 'active' ? (job.allocation ?? 0) : 0);
  $: total = jobs.reduce((sum, job) => sum + (Number(values[job.id]) || 0), 0);
  $: invalid = jobs.some((job) => {
    const v = values[job.id];
    return !Number.isInteger(v) || v < 0 || v > 100;
  });
  $: changed = jobs.filter((job) => Number(values[job.id]) !== current(job));

  /** Every job's share, the same whole number each, the rest to the first. */
  function shareEqually() {
    const running = jobs.filter((job) => Number(values[job.id]) > 0);
    const among = running.length ? running : jobs;
    if (!among.length) return;
    const each = Math.floor(100 / among.length);
    let left = 100 - each * among.length;
    const next = { ...values };
    for (const job of jobs) next[job.id] = 0;
    for (const job of among) {
      next[job.id] = each + (left > 0 ? 1 : 0);
      if (left > 0) left -= 1;
    }
    values = next;
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
      between them; set them all here and save once. Above 0% a job is active; at 0% it is
      inactive. Completed jobs cannot run again and are not listed.
    </p>
  </div>

  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  {#if saved}<p class="text-sm text-success" role="status">{saved}</p>{/if}

  {#if !loaded && !error}
    <p class="text-muted-foreground">Loading…</p>
  {:else if loaded && !jobs.length}
    <p class="text-muted-foreground">No job is active or inactive: there is nothing to allocate.</p>
  {:else if loaded}
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
            {#each jobs as job (job.id)}
              <tr class:font-medium={Number(values[job.id]) !== current(job)}>
                <td><a href="/admin/jobs/{job.id}">{jobTitle(job)}</a></td>
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
        "Share equally" splits 100% among the jobs set above 0% (all of them when none is).
        Jobs you leave unchanged are not sent.
      </p>
    </form>
  {/if}
</div>
