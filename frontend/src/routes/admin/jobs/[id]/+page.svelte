<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import {
    api,
    ApiError,
    type ArtifactRebuild,
    type DataGap,
    type JobExport,
    type JobStats
  } from '$lib/api';
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
  let rebuild: ArtifactRebuild[] | null = null;
  let jobExport: JobExport | null = null;
  let exportPoll: number | undefined;
  // Set when the page goes, so a poll whose request was in flight then does
  // not schedule another one on a page nobody has open.
  let destroyed = false;

  async function reload() {
    stats = await api.job(jobId);
    if (stats.job.allocation !== null) allocation = stats.job.allocation;
    gaps = await api.jobDataGaps(jobId);
    await loadExport();
  }

  // The export is built on a background task, so the page polls while one is
  // running. A job that has never been exported answers 404, which is not an
  // error worth showing.
  async function loadExport() {
    window.clearTimeout(exportPoll);
    try {
      jobExport = await api.jobExport(jobId);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) jobExport = null;
      else throw e;
    }
    if (jobExport?.state === 'running' && !destroyed) {
      exportPoll = window.setTimeout(() => loadExport().catch((e) => (error = e.message)), 3000);
    }
  }

  async function startExport() {
    error = '';
    notice = '';
    try {
      await api.startExport(jobId);
      await loadExport();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  function megabytes(bytes: number | null): string {
    return bytes === null ? '—' : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  onMount(() => {
    reload().catch((e) => (error = e.message));
    const unsubscribe = subscribeToJob<JobStats>(jobId, (value) => (stats = value));
    return () => {
      destroyed = true;
      window.clearTimeout(exportPoll);
      unsubscribe();
    };
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

  // Leave-generation KLVs are derivable from the results they were built from,
  // so a lost object is repairable without a restore. Rebuilding also answers
  // whether each object still holds the bytes recorded when the generation
  // closed. See PLAN.md, "Artifacts: back up, or rebuild?".
  async function rebuildArtifacts(force = false) {
    // Forcing replaces every generation's object with what the database
    // rebuilds now -- including ones that differ because the results moved
    // on after the generation closed, which is the KLV workers played.
    if (
      force &&
      !confirm(
        'Rewrite every generation\'s KLV from the database? Every object is replaced, ' +
          'matching or not, and after a MAGPIE builder change they all differ. ' +
          'The replaced versions stay in the bucket as noncurrent versions. ' +
          'The job must be deactivated first.'
      )
    ) {
      return;
    }
    error = '';
    notice = '';
    rebuild = null;
    try {
      rebuild = await api.rebuildArtifacts(jobId, force);
      const missing = rebuild.filter((r) => r.rewritten).length;
      // A generation written by a different builder is expected to differ:
      // MAGPIE builds these now, so an upgrade legitimately changes the bytes.
      // Counting it as drift would make every upgrade read as data loss.
      const drifted = rebuild.filter((r) => !r.matches && r.same_builder).length;
      const rebuilt = rebuild.filter((r) => !r.same_builder).length;
      // Left for an admin: workers refuse these until one is resolved.
      const foreign = rebuild.filter((r) => !r.object_accounted_for && !r.rewritten).length;
      notice =
        `Checked ${rebuild.length} generations: ${missing} rewritten, ` +
        `${drifted} differing from the recorded hash` +
        (rebuilt > 0 ? `, ${rebuilt} built by a different MAGPIE builder` : '') +
        (foreign > 0
          ? `; ${foreign} holding bytes nothing here accounts for, which workers refuse — restore the right object version or force a rebuild.`
          : '.');
    } catch (e) {
      error = (e as Error).message;
    }
  }

  // Both are one click away from Activate and neither can be taken back: a
  // purge deletes every task, claim and result the job holds, and a completed
  // job can never be reactivated. Delete already asked; these did not.
  function purge() {
    if (
      !confirm(
        'Purge this job? Every task, claim and result it holds is deleted and the job starts over (a completed job returns to inactive, to be activated again). This cannot be undone.'
      )
    )
      return;
    run(() => api.purgeJob(jobId), 'Results purged; the job starts over from its first task.');
  }

  function forceComplete() {
    if (
      !confirm(
        'Force-complete this job? It stops dispatching for good: a completed job cannot be reactivated.'
      )
    )
      return;
    run(() => api.completeJob(jobId), 'Job force-completed.');
  }

  // Accepted leave results are staged and merged into the per-rack totals in
  // batches; this merges now, for an admin who wants the rack figures current.
  async function mergeProgress() {
    error = '';
    notice = '';
    try {
      const merged = await api.mergeLeaveProgress(jobId);
      await reload();
      notice =
        `Merged ${merged.folds_merged.toLocaleString()} staged results into ` +
        `${merged.racks_updated.toLocaleString()} racks.`;
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
        <button class="btn-secondary" on:click={forceComplete}>Force complete</button>
        <button class="btn-secondary" on:click={purge}>Purge results</button>
        {#if stats.job.job_type === 'leave_generation'}
          <button class="btn-secondary" on:click={() => rebuildArtifacts()}>Check artifacts</button>
          <button class="btn-destructive" on:click={() => rebuildArtifacts(true)}>Force rebuild</button>
          <button
            class="btn-secondary"
            title="Fold staged results into the rack totals now, rather than at the next half-hourly merge"
            on:click={mergeProgress}
          >
            Merge progress now
          </button>
        {/if}
        <button class="btn-destructive" on:click={remove}>Delete job</button>
      </div>
      <p class="text-xs text-muted-foreground">
        The active jobs may allocate at most 100% between them; activation is rejected if this
        job's share would push the total over. A share of 0% is the same as inactive: the job
        is offered to nobody until it is raised.
      </p>

      {#if rebuild}
        <div class="overflow-x-auto">
          <table class="table">
            <thead>
              <tr>
                <th>Generation</th>
                <th>Object</th>
                <th>Hash</th>
                <th>Served</th>
                <th>Action</th>
              </tr>
            </thead>
            <tbody>
              {#each rebuild as row}
                <tr>
                  <td class="tabular-nums">{row.generation}</td>
                  <td class={row.object_present ? '' : 'text-destructive'}>
                    {row.object_present ? 'present' : 'missing'}
                  </td>
                  <td
                    class={row.matches || !row.same_builder ? '' : 'text-destructive'}
                    title={row.stored_sha256}
                  >
                    {#if !row.same_builder}
                      built by {row.stored_builder}, rebuilt by {row.rebuilt_builder}
                    {:else}
                      {row.matches ? 'matches' : 'differs from the recorded hash'}
                    {/if}
                  </td>
                  <td
                    class="font-mono text-xs {row.object_accounted_for || row.rewritten
                      ? ''
                      : 'text-destructive'}"
                    title={row.object_sha256 ?? 'missing'}
                  >
                    {row.served_sha256.slice(0, 12)}{#if !row.object_accounted_for && !row.rewritten}
                      <span class="font-sans">— the object holds {row.object_sha256?.slice(0, 12)}, which nothing accounts for</span>{/if}
                  </td>
                  <td>{row.rewritten ? 'rewritten' : 'left alone'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <p class="text-xs text-muted-foreground">
          A missing object is rebuilt from the job's rack progress. A differing hash is not: the
          results have almost certainly moved on since the generation closed, and rewriting on that
          basis would replace the KLV workers actually played with by one they never saw.
        </p>
        <p class="text-xs text-muted-foreground">
          A generation built by a different MAGPIE builder is expected to differ and is not
          drift. MAGPIE builds these KLVs, so an upgrade can legitimately change the bytes for
          the same leave values; only two artifacts from the <em>same</em> builder disagreeing
          is evidence of anything.
        </p>
      {/if}
    </div>

    {#if stats.job.status === 'completed'}
      <div class="card space-y-3">
        <h2 class="text-lg font-medium">Export</h2>
        <p class="text-xs text-muted-foreground">
          A completed job's whole corpus as one gzipped NDJSON file, built once on a background
          task and downloaded straight from the artifact store. An opening-rack line is a rack
          with its ranked moves; a games job that captured positions gets those as a second
          file. Refused while the job's last claims are still in flight.
        </p>
        <div class="flex flex-wrap items-center gap-3">
          <button
            class="btn-secondary"
            on:click={startExport}
            disabled={jobExport?.state === 'running'}
          >
            {jobExport ? 'Export again' : 'Export results'}
          </button>
          {#if jobExport}
            <span class="text-sm">
              {#if jobExport.state === 'running'}
                Building…
              {:else if jobExport.state === 'ready'}
                {(jobExport.row_count ?? 0).toLocaleString()} rows ·
                {megabytes(jobExport.bytes)}
                {#if jobExport.download_url}
                  · <a href={jobExport.download_url}>download</a>
                {/if}
                {#if jobExport.positions_row_count !== null}
                  · {jobExport.positions_row_count.toLocaleString()} captured positions ·
                  {megabytes(jobExport.positions_bytes)}
                  {#if jobExport.positions_download_url}
                    · <a href={jobExport.positions_download_url}>download positions</a>
                  {/if}
                {/if}
                {#if jobExport.download_url}(links valid for an hour){/if}
              {:else if jobExport.state === 'expired'}
                Expired: the store keeps an export for thirty days. Export again to rebuild it.
              {:else}
                <span class="text-destructive">Failed: {jobExport.error ?? 'unknown error'}</span>
              {/if}
            </span>
          {/if}
        </div>
      </div>
    {/if}

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">Progress</h2>
      {#if stats.games}
        <ProgressBar
          value={stats.games.units_completed}
          max={stats.games.max_units}
          label="{stats.games.unit}s completed"
        />
        <p class="text-sm text-muted-foreground">
          {#if stats.games.decided}
            SPRT {sprtLabel(stats.games.decided.status)} at LLR
            {stats.games.decided.llr.toFixed(3)} (now {stats.games.sprt.llr.toFixed(3)})
          {:else}
            SPRT {sprtLabel(stats.games.sprt.status)} — LLR {stats.games.sprt.llr.toFixed(3)}
          {/if}
        </p>
      {:else if stats.opening_racks}
        <ProgressBar
          value={stats.opening_racks.racks_analyzed}
          max={stats.opening_racks.racks_total}
          label="racks analysed"
        />
      {:else if stats.leave_generation}
        <ProgressBar
          value={stats.leave_generation.generations_closed}
          max={stats.leave_generation.generation_count}
          label="generations closed"
        />
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
