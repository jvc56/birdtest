<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api, type DataGap, type JobStats } from '$lib/api';
  import { subscribeToJob } from '$lib/sse';
  import { jobTypeLabel, sprtLabel, duration } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';
  import ProgressBar from '$lib/components/ProgressBar.svelte';
  import WorkerTable from '$lib/components/WorkerTable.svelte';

  // The [id] route only matches when the param is present.
  const jobId = $page.params.id as string;

  let stats: JobStats | null = null;
  let gaps: DataGap[] = [];
  let allocation = 100;
  let error = '';
  let notice = '';

  async function reload() {
    stats = await api.job(jobId);
    if (stats.job.allocation !== null) allocation = stats.job.allocation;
    gaps = await api.jobDataGaps(jobId);
  }

  onMount(() => {
    reload().catch((e) => (error = e.message));
    return subscribeToJob<JobStats>(jobId, (value) => (stats = value));
  });

  async function run(action: () => Promise<unknown>, message: string) {
    error = '';
    notice = '';
    try {
      await action();
      notice = message;
      await reload();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function remove() {
    if (!confirm('Delete this job and every task and result it holds? This cannot be undone.'))
      return;
    try {
      await api.deleteJob(jobId);
      goto('/jobs');
    } catch (e) {
      error = (e as Error).message;
    }
  }
</script>

{#if error}<p class="mb-4 text-destructive">{error}</p>{/if}
{#if notice}<p class="mb-4 text-success">{notice}</p>{/if}

{#if !stats}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="space-y-6">
    <header class="flex flex-wrap items-center gap-3">
      <h1 class="text-2xl font-semibold">{jobTypeLabel(stats.job.job_type)}</h1>
      <JobStatusBadge status={stats.job.status} />
      <a href="/jobs/{jobId}" class="text-sm">public view</a>
    </header>

    <div class="card space-y-4">
      <h2 class="text-lg font-medium">Controls</h2>
      <div class="flex flex-wrap items-end gap-3">
        <div>
          <label class="label" for="alloc">Allocation %</label>
          <input id="alloc" type="number" min="0" max="100" class="input w-28" bind:value={allocation} />
        </div>
        <button
          class="btn-primary"
          on:click={() => run(() => api.activateJob(jobId, allocation), 'Job activated.')}
        >
          Activate
        </button>
        <button
          class="btn-secondary"
          on:click={() => run(() => api.deactivateJob(jobId), 'Job deactivated.')}
        >
          Deactivate
        </button>
        <button
          class="btn-secondary"
          on:click={() => run(() => api.completeJob(jobId), 'Job force-completed.')}
        >
          Force complete
        </button>
        <button
          class="btn-secondary"
          on:click={() =>
            run(() => api.purgeJob(jobId), 'Results purged; tasks returned to available.')}
        >
          Purge results
        </button>
        <button class="btn-destructive" on:click={remove}>Delete job</button>
      </div>
      <p class="text-xs text-muted-foreground">
        Active jobs in a priority tier must allocate 100% between them; activation is rejected if
        this job's share would push the tier over.
      </p>
    </div>

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">Progress</h2>
      {#if stats.games}
        <ProgressBar
          value={stats.games.units_completed}
          max={stats.games.max_units}
          label="{stats.games.unit}s completed"
        />
        <p class="text-sm text-muted-foreground">
          SPRT {sprtLabel(stats.games.sprt.status)} — LLR {stats.games.sprt.llr.toFixed(3)}
        </p>
      {:else}
        <ProgressBar value={stats.tasks_completed} max={stats.tasks_total} label="tasks completed" />
      {/if}
      <p class="text-sm text-muted-foreground">
        {stats.tasks_available.toLocaleString()} available ·
        {stats.tasks_claimed.toLocaleString()} claimed ·
        ETA {duration(stats.eta_seconds)}
      </p>
    </div>

    <div class="card">
      <h2 class="mb-1 text-lg font-medium">Data gaps</h2>
      <p class="mb-3 text-sm text-muted-foreground">
        Files workers reported they could not match when they declined this job. A job pinned to
        data nobody has yet gets nothing done and says nothing about it; this is what turns that
        absence into a statement. The fix is an admin decision — wait for the MAGPIE release that
        installs the data, or pin the job to the older rows.
      </p>
      {#if gaps.length}
        <table class="table">
          <thead>
            <tr>
              <th>File</th><th>Expected digest</th>
              <th class="text-right">Workers</th><th class="text-right">Declines</th>
            </tr>
          </thead>
          <tbody>
            {#each gaps as gap}
              <tr>
                <td>{gap.role} {gap.name}</td>
                <td class="font-mono text-xs">{gap.expected.slice(0, 12)}</td>
                <td class="text-right tabular-nums">{gap.workers}</td>
                <td class="text-right tabular-nums">{gap.declines}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="text-sm text-muted-foreground">No worker has declined this job.</p>
      {/if}
    </div>

    <div class="card">
      <h2 class="mb-3 text-lg font-medium">Contributors</h2>
      <WorkerTable workers={stats.workers} />
    </div>
  </div>
{/if}
