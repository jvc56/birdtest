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
  import { jobTitle, jobTypeLabel, sprtLabel, sprtState, duration } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';
  import CompletionNote from '$lib/components/CompletionNote.svelte';
  import ProgressBar from '$lib/components/ProgressBar.svelte';
  import WorkerTable from '$lib/components/WorkerTable.svelte';

  // The [id] route only matches when the param is present.
  const jobId = $page.params.id as string;

  // How this page's state is kept (redesigned in the audit's pass 17, after
  // three passes of patches to it):
  //
  // - `stats` comes from two sources, the REST read and the live stream,
  //   and neither carries an order. A REST read is applied only if it is the
  //   newest one started and no stream payload arrived while it was out: a
  //   read that landed late put back a status the stream had already moved
  //   past, and an inactive or completed job sends nothing to correct it.
  // - The allocation box is the admin's alone. It is filled once, from the
  //   first payload that says the job's allocation, and no read touches it
  //   after that; the job's current allocation is shown beside it. (Each
  //   earlier attempt to keep the two in step lost a value the admin had
  //   typed, and Activate sent the old one.)
  // - The job, its data gaps and its export are read apart, so a failure of
  //   one shows as that and not as an empty answer, and failed reads are
  //   tried again on the next live payload and every five seconds.
  let stats: JobStats | null = null;
  // null until read: shown as "could not load", never as "none".
  let gaps: DataGap[] | null = null;
  let allocation: number | null = null;
  let allocationFilled = false;
  let reloadGen = 0;
  let streamPayloads = 0;
  // Why the page's reads last failed, if they did; kept apart from `error`
  // (an action's) so a later successful read can clear it.
  let loadError = '';
  let exportError = '';
  // The job is gone (a 404): nothing to retry.
  let gone = false;
  let reloading = false;
  let retry: number | undefined;
  let busy = false;
  let error = '';
  let notice = '';
  let rebuild: ArtifactRebuild[] | null = null;
  let jobExport: JobExport | null = null;
  // An export started here that no read has shown yet: the button stays off
  // (its read failing left "Export results" up, and a second click a 409).
  let exportStarted = false;
  // Bumped by an export start: a reload's export read that began before it
  // is older than the start, and would put "Export results" back.
  let exportGen = 0;
  let exportPoll: number | undefined;
  // Set when the page goes, so a request in flight then schedules nothing.
  let destroyed = false;

  // A 404 from anything -- a read, an action, the stream -- means the job was
  // deleted, here or elsewhere: say so, and offer nothing more. (Only a read
  // noticed before, so a job deleted by another admin kept every action.)
  function goneIf(e: unknown): boolean {
    if (e instanceof ApiError && e.status === 404) markGone();
    return gone;
  }

  // An action's 404 is asked about first: one could come from something the
  // action touched (an object the rebuild reads), not the job, and "gone"
  // disables every action until a reload.
  function goneIfConfirmed(e: unknown): boolean {
    if (!(e instanceof ApiError && e.status === 404)) return false;
    api.job(jobId).catch((read) => goneIf(read));
    return false;
  }

  function markGone() {
    gone = true;
    loadError = 'This job no longer exists.';
    notice = '';
    error = '';
    exportError = '';
    window.clearTimeout(retry);
    window.clearTimeout(exportPoll);
  }

  function fillAllocation(value: JobStats) {
    if (allocationFilled) return;
    allocation = value.job.allocation ?? 100;
    allocationFilled = true;
  }

  async function reload() {
    const gen = ++reloadGen;
    const exportGenAtStart = exportGen;
    const payloadsAtStart = streamPayloads;
    reloading = true;
    window.clearTimeout(retry);
    const [job, gapsRead, exportRead] = await Promise.allSettled([
      api.job(jobId),
      api.jobDataGaps(jobId),
      fetchExport()
    ]);
    if (gen !== reloadGen || destroyed) return;
    reloading = false;
    const failures: string[] = [];
    if (job.status === 'fulfilled') {
      if (streamPayloads === payloadsAtStart) stats = job.value;
      fillAllocation(job.value);
    } else if (!goneIf(job.reason)) failures.push((job.reason as Error).message);
    if (gapsRead.status === 'fulfilled') gaps = gapsRead.value;
    else if (!gone) failures.push((gapsRead.reason as Error).message);
    if (exportRead.status === 'fulfilled') {
      if (exportGen === exportGenAtStart) applyExport(exportRead.value);
    } else if (!gone && exportGen === exportGenAtStart) exportError = `Could not check the export: ${(exportRead.reason as Error).message}`;
    if (gone) return;
    loadError = failures.length ? `Could not load all of this job: ${[...new Set(failures)].join('; ')}` : '';
    // And again in a few seconds: an inactive job sends no live payload.
    if (!gone && (failures.length || exportRead.status === 'rejected')) {
      retry = window.setTimeout(() => !reloading && reload(), 5000);
    }
  }

  // The export is built on a background task, so the page polls while one is
  // running. A job that has never been exported answers 404, which is not an
  // error worth showing.
  async function fetchExport(): Promise<JobExport | null> {
    try {
      return await api.jobExport(jobId);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }
  }

  function applyExport(value: JobExport | null) {
    jobExport = value;
    exportStarted = false;
    exportError = '';
    if (value?.state === 'running') pollExport();
  }

  // One poll at a time; a failed one polls again (one that stopped left
  // "Building…" up after the export was ready).
  function pollExport() {
    window.clearTimeout(exportPoll);
    if (destroyed || gone) return;
    exportPoll = window.setTimeout(async () => {
      try {
        const value = await fetchExport();
        // An export being polled cannot vanish unless its job did: a 404 here
        // reads as "never exported" to fetchExport, so ask the job.
        if (value === null && jobExport !== null) api.job(jobId).catch((read) => goneIf(read));
        applyExport(value);
      } catch (e) {
        if (goneIf(e)) return;
        exportError = `Could not check the export: ${(e as Error).message}`;
        pollExport();
      }
    }, 3000);
  }

  async function startExport() {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      await api.startExport(jobId);
      exportGen += 1;
      exportStarted = true;
    } catch (e) {
      if (!goneIfConfirmed(e)) error = (e as Error).message;
      busy = false;
      return;
    }
    // Started: a read that fails now is the export's to retry, not the
    // action's error (it left "Export results" up, and a second click a 409).
    try {
      applyExport(await fetchExport());
    } catch (e) {
      exportError = `Could not check the export: ${(e as Error).message}`;
      pollExport();
    } finally {
      busy = false;
    }
  }

  function megabytes(bytes: number | null): string {
    return bytes === null ? '—' : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  onMount(() => {
    reload();
    const unsubscribe = subscribeToJob<JobStats>(
      jobId,
      (value) => {
        streamPayloads += 1;
        stats = value;
        fillAllocation(value);
        if ((loadError || exportError) && !reloading && !gone) reload();
      },
      (status) => status === 404 && markGone()
    );
    return () => {
      destroyed = true;
      window.clearTimeout(exportPoll);
      window.clearTimeout(retry);
      unsubscribe();
    };
  });

  function activate() {
    const value = allocation;
    if (value === null || !Number.isInteger(Number(value)) || value < 0 || value > 100) {
      notice = '';
      error = 'Enter a whole-number allocation from 0 to 100.';
      return;
    }
    run(() => api.activateJob(jobId, Number(value)), 'Job activated.');
  }

  // One action at a time: a double click sent two activations, or rebuilt
  // every generation twice.
  async function run(action: () => Promise<unknown>, message: string) {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      await action();
      notice = message;
      await reload();
    } catch (e) {
      if (!goneIfConfirmed(e)) error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  // Leave-generation KLVs are derivable from the results they were built from,
  // so a lost object is repairable without a restore. Rebuilding also answers
  // whether each object still holds the bytes recorded when the generation
  // closed. See PLAN.md, "Artifacts: back up, or rebuild?".
  async function rebuildArtifacts(force = false) {
    if (busy) return;
    // Forcing replaces every generation's object with what the database
    // rebuilds now -- including ones that differ. A closed generation's rows
    // do not move (a late result is credited, not folded), so a difference
    // means the object or the rows were damaged or replaced, and the object
    // may be the KLV workers played.
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
    busy = true;
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
      if (!goneIfConfirmed(e)) error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  // Both are one click away from Activate and neither can be taken back: a
  // purge deletes every task, claim and result the job holds, and a completed
  // job can never be reactivated. Delete already asked; these did not.
  function purge() {
    if (busy) return;
    if (
      !confirm(
        'Purge this job? Every task, claim and result it holds is deleted and the job starts over (a completed job returns to inactive, to be activated again). This cannot be undone.'
      )
    )
      return;
    // Only an active job goes on running: a completed one returns to inactive
    // and an inactive one stays so. Said as "starts over", an admin who had
    // purged a completed job watched it do nothing.
    const running = stats?.job.status === 'active';
    run(
      () => api.purgeJob(jobId),
      running
        ? 'Results purged; the job starts over from its first task.'
        : 'Results purged. The job is inactive: activate it to start over from its first task.'
    );
  }

  function forceComplete() {
    if (busy) return;
    if (
      !confirm(
        'Force-complete this job? It stops dispatching: a completed job cannot be reactivated (only a purge, which deletes its results, starts it over).'
      )
    )
      return;
    run(() => api.completeJob(jobId), 'Job force-completed.');
  }

  // Accepted leave results are staged and merged into the per-rack totals in
  // batches; this merges now, for an admin who wants the rack figures current.
  async function mergeProgress() {
    if (busy) return;
    busy = true;
    error = '';
    notice = '';
    try {
      const merged = await api.mergeLeaveProgress(jobId);
      await reload();
      notice =
        `Merged ${merged.folds_merged.toLocaleString()} staged results into ` +
        `${merged.racks_updated.toLocaleString()} racks.`;
    } catch (e) {
      if (!goneIfConfirmed(e)) error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (busy) return;
    if (!confirm('Delete this job and every task and result it holds? This cannot be undone.'))
      return;
    busy = true;
    error = '';
    notice = '';
    try {
      await api.deleteJob(jobId);
      goto('/jobs');
    } catch (e) {
      if (!goneIfConfirmed(e)) error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

{#if error}<p class="mb-4 text-destructive">{error}</p>{/if}
{#if loadError}<p class="mb-4 text-destructive">{loadError}</p>{/if}
{#if exportError}<p class="mb-4 text-destructive">{exportError}</p>{/if}
{#if notice}<p class="mb-4 text-success">{notice}</p>{/if}

{#if !stats}
  {#if !gone}<p class="text-muted-foreground">Loading…</p>{/if}
{:else}
  <div class="space-y-6">
    <header class="flex flex-wrap items-center gap-3">
      <h1 class="text-2xl font-semibold">{jobTitle(stats.job)}</h1>
      {#if stats.job.name}
        <span class="text-sm text-muted-foreground">{jobTypeLabel(stats.job.job_type)}</span>
      {/if}
      <JobStatusBadge status={stats.job.status} />
      <a href="/jobs/{jobId}" class="text-sm">public view</a>
    </header>
    <CompletionNote {stats} />

    <div class="card space-y-4">
      <h2 class="text-lg font-medium">Controls</h2>
      <p class="text-xs text-muted-foreground">
        An allocation is a share of claims, not of worker time: a job whose tasks take longer holds
        more of the fleet than its share.
      </p>
      <div class="flex flex-wrap items-end gap-3">
        <div>
          <label
            class="label"
            for="alloc"
            title="A share of claims, not of worker time: a job whose tasks take longer holds more of the fleet than its share (PLAN KL-88)."
          >
            Allocation % of claims
          </label>
          <input
            id="alloc"
            type="number"
            min="0"
            max="100"
            class="input w-28"
            bind:value={allocation}
            on:input={() => (allocationFilled = true)}
          />
          <p class="mt-1 text-xs text-muted-foreground">
            {#if stats.job.allocation === null}
              Set: none
            {:else if stats.job.status === 'active'}
              Now: {stats.job.allocation}%
            {:else if stats.job.status === 'completed'}
              Was: {stats.job.allocation}% (completed)
            {:else}
              Set: {stats.job.allocation}% (offered to nobody while {stats.job.status})
            {/if}
          </p>
        </div>
        <!-- A completed job is final: the server refuses all three (409). -->
        <button
          class="btn-primary"
          disabled={busy || gone || stats.job.status === 'completed'}
          on:click={activate}
        >
          Activate
        </button>
        <button
          class="btn-secondary"
          disabled={busy || gone || stats.job.status === 'completed'}
          on:click={() => run(() => api.deactivateJob(jobId), 'Job deactivated.')}
        >
          Deactivate
        </button>
        <button
          class="btn-secondary"
          disabled={busy || gone || stats.job.status === 'completed'}
          on:click={forceComplete}
        >
          Force complete
        </button>
        <button class="btn-secondary" disabled={busy || gone} on:click={purge}>Purge results</button>
        {#if stats.job.job_type === 'leave_generation'}
          <button class="btn-secondary" disabled={busy || gone} on:click={() => rebuildArtifacts()}>Check artifacts</button>
          <button class="btn-destructive" disabled={busy || gone} on:click={() => rebuildArtifacts(true)}>Force rebuild</button>
          <button
            class="btn-secondary"
            title="Fold staged results into the rack totals now, rather than at the next half-hourly merge"
            disabled={busy || gone}
            on:click={mergeProgress}
          >
            Merge progress now
          </button>
        {/if}
        <button class="btn-destructive" disabled={busy || gone} on:click={remove}>Delete job</button>
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
          A missing object is rebuilt from the job's rack progress. A differing hash is not: a closed
          generation's rows do not change, so a difference means the object or the rows were damaged
          or replaced (a restore from another point, a hand edit). Find out which before forcing a
          rebuild, which would replace the KLV workers actually played with.
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
            disabled={busy || gone || exportStarted || jobExport?.state === 'running'}
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
                {#if jobExport.sha256}
                  · <span class="whitespace-nowrap">SHA-256 of the .gz</span>
                  <code class="break-all text-xs">{jobExport.sha256}</code>
                {/if}
                {#if jobExport.positions_row_count !== null}
                  · {jobExport.positions_row_count.toLocaleString()} captured positions ·
                  {megabytes(jobExport.positions_bytes)}
                  {#if jobExport.positions_download_url}
                    · <a href={jobExport.positions_download_url}>download positions</a>
                  {/if}
                  {#if jobExport.positions_sha256}
                    · <span class="whitespace-nowrap">SHA-256 of the .gz</span>
                    <code class="break-all text-xs">{jobExport.positions_sha256}</code>
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
            SPRT {sprtLabel(stats.games.decided.status)}, LLR
            {stats.games.decided.llr.toFixed(3)} (now {stats.games.sprt.llr.toFixed(3)})
          {:else}
            SPRT {sprtLabel(sprtState(stats.job.status, stats.games))} — LLR {stats.games.sprt.llr.toFixed(3)}
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
      {#if gaps === null}
        <p class="text-sm text-muted-foreground">
          {loadError ? 'Could not load the data gaps.' : 'Loading…'}
        </p>
      {:else if gaps.length}
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
