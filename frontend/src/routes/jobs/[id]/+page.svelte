<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { api, type JobStats, type RackLookupRow } from '$lib/api';
  import { subscribeToJob } from '$lib/sse';
  import { session } from '$lib/auth';
  import { datetime, jobTypeLabel, jobTitle } from '$lib/format';
  import JobStatusCard from '$lib/components/JobStatusCard.svelte';
  import JobStatsRow from '$lib/components/JobStatsRow.svelte';
  import TaskCounts from '$lib/components/TaskCounts.svelte';
  import WorkerTable from '$lib/components/WorkerTable.svelte';
  import ProgressBar from '$lib/components/ProgressBar.svelte';
  import JobSettings from '$lib/components/JobSettings.svelte';
  import MatchScore from '$lib/components/MatchScore.svelte';
  import MatchTestCard from '$lib/components/MatchTestCard.svelte';
  import SavedPositions from '$lib/components/SavedPositions.svelte';
  import { playersLine, type JobConfig } from '$lib/jobSettings';
  import { analysesPerRack, rackConsensus } from '$lib/consensus';
  import { plyAt, plyColumns, plyHeaders, showsIterations } from '$lib/moveList';

  // The [id] route only matches when the param is present.
  const jobId = $page.params.id as string;

  let stats: JobStats | null = null;
  let error = '';
  // Fixed once the job exists: read once. Without it the page still shows.
  let config: JobConfig | null = null;

  // Opening-rack search
  let rackQuery = '';
  let rackMoves: RackLookupRow[] | null = null;
  $: lookupConsensus = rackMoves ? rackConsensus(rackMoves) : null;
  $: lookupWinPct = rackMoves?.some((m) => m.win_percentage !== null) ?? false;
  $: lookupPlies = rackMoves ? plyColumns(rackMoves) : 0;
  $: lookupIters = rackMoves ? showsIterations(rackMoves) : false;
  $: seeksConsensus = (config?.opening_racks?.max_results_per_rack ?? 1) > 1;
  let rackError = '';
  // A few racks the job has analysed, to try the search on: the newest, from
  // the results feed. Read once, when the page knows it is an opening-rack job.
  let sampleRacks: string[] = [];
  let samplesRequested = false;
  $: if (stats?.opening_racks && !samplesRequested) {
    samplesRequested = true;
    api
      .jobResults(jobId, { per_page: 50 })
      // One record per rack per accepted claim, so a rack can repeat.
      .then((page) => (sampleRacks = [...new Set(page.items.map((r) => String(r.rack)))].slice(0, 10)))
      .catch(() => (sampleRacks = []));
  }

  onMount(() => {
    api
      .jobConfig(jobId)
      .then((value) => (config = value))
      .catch(() => (config = null));
    api
      .job(jobId)
      .then((value) => (stats = value))
      // Only while there is nothing to show: a late failure after the stream
      // has delivered stats would otherwise hide them for good on a job no
      // longer changing.
      .catch((e) => {
        if (stats === null) error = e.message;
      });
    // The stream carries the same payload as the REST call, so an update is a
    // straight replacement rather than a merge.
    // A live payload also clears an error the first load hit (a deploy's
    // 503): the page otherwise stayed on the error with the stats arriving
    // behind it.
    return subscribeToJob<JobStats>(jobId, (value) => {
      stats = value;
      error = '';
    });
  });

  async function lookupRack() {
    rackError = '';
    rackMoves = null;
    try {
      const result = await api.rackLookup(jobId, rackQuery);
      rackMoves = result.items;
      if (!rackMoves.length) rackError = 'No analysis stored for that rack yet.';
    } catch (e) {
      rackError = (e as Error).message;
    }
  }
</script>

{#if error}
  <p class="text-destructive">{error}</p>
{:else if !stats}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="space-y-6">
    <header class="flex flex-wrap items-center gap-3">
      <h1 class="text-2xl font-semibold">{jobTitle(stats.job)}</h1>
      {#if stats.job.name}
        <span class="text-sm text-muted-foreground">{jobTypeLabel(stats.job.job_type)}</span>
      {/if}
      <span class="text-sm text-muted-foreground">
        {stats.job.lexicon ?? '—'} · {stats.job.variant ?? '—'}{#if config?.players.length}
          · {playersLine(config)}{/if}
      </span>
      <!-- The admin page (activate, purge, export, artifacts) was reachable
           only by the redirect after creating the job. -->
      {#if $session?.is_admin}
        <a href="/admin/jobs/{stats.job.id}" class="btn-secondary ml-auto no-underline">Manage</a>
      {/if}
    </header>
    <JobStatusCard {stats} />

    <JobStatsRow {stats} />

    <div class="card space-y-4">
      <h2 class="text-lg font-medium">Progress</h2>
      {#if stats.games}
        <ProgressBar
          value={stats.games.units_completed}
          max={stats.games.max_units}
          label="{stats.games.unit}s completed"
        />
      {:else if stats.opening_racks}
        <!-- Tasks are made on demand, so a task count is only what has been
             handed out so far: a job 1% through its racks read 99%. A rack is
             done once settled, which for a consensus job may take several
             analyses. -->
        <ProgressBar
          value={stats.opening_racks.racks_settled}
          max={stats.opening_racks.racks_total}
          label="racks settled"
        />
      {:else if stats.leave_generation}
        <ProgressBar
          value={stats.leave_generation.generations_closed}
          max={stats.leave_generation.generation_count}
          label="generations closed"
        />
      {:else}
        <ProgressBar
          value={stats.tasks_completed}
          max={stats.tasks_total}
          label="tasks completed"
        />
      {/if}
      <TaskCounts {stats} />
      <p class="text-xs text-muted-foreground">
        Created by <span class="break-all">{stats.job.created_by ?? 'unknown'}</span>, {datetime(
          stats.job.created_at
        )}{#if stats.job.min_magpie_version}
          · requires MAGPIE ≥ {stats.job.min_magpie_version}{/if}
      </p>
    </div>

    {#if config}
      <JobSettings {config} />
    {/if}

    {#if stats.games}
      <MatchScore games={stats.games} players={config?.players.map((p) => p.name) ?? []} />
    {/if}
    <MatchTestCard {stats} players={config?.players.map((p) => p.name) ?? []} />

    {#if config?.games?.capture_positions}
      {#if $session}
        <SavedPositions
          {jobId}
          players={config.players.map((p) => p.name)}
          paired={config.job.job_type === 'game_pairs'}
          firstDivergence={config.games.capture_first_divergence}
          progress={stats.games?.units_completed ?? 0}
        />
      {:else if $session === null}
        <div class="card space-y-1">
          <h2 class="text-lg font-medium">Saved positions</h2>
          <p class="text-sm text-muted-foreground">
            {#if config.games.capture_first_divergence}
              This job keeps the turn where each game pair's two games first diverged.
            {:else}
              This job keeps the position analysed on every turn of its games.
            {/if}
            <a href="/login?next={encodeURIComponent(`/jobs/${jobId}`)}">Sign in</a> to search them.
          </p>
        </div>
      {/if}
    {/if}

    <!-- Ratings are pool-scoped and live on /ratings: a rating is a statement
         about a player config across every pair it has played, not something
         one job owns. -->

    {#if stats.opening_racks}
      <div class="card space-y-4">
        <h2 class="text-lg font-medium">Opening racks</h2>
        <dl class="grid grid-cols-2 gap-4 text-sm sm:grid-cols-3">
          <div>
            <dt class="text-muted-foreground">Racks analyzed</dt>
            <dd class="text-xl tabular-nums">
              {stats.opening_racks.racks_analyzed.toLocaleString()}
              <span class="text-sm text-muted-foreground">
                / {stats.opening_racks.racks_total.toLocaleString()}
              </span>
            </dd>
          </div>
          {#if seeksConsensus}
            <div>
              <dt class="text-muted-foreground" title="Agreed on, or analysed the most times the job allows">
                Racks settled
              </dt>
              <dd class="text-xl tabular-nums">{stats.opening_racks.racks_settled.toLocaleString()}</dd>
            </div>
            <div>
              <dt class="text-muted-foreground" title="Analysed the most times the job allows, their analyses still split">
                Settled without a consensus
              </dt>
              <dd class="text-xl tabular-nums">
                {stats.opening_racks.racks_without_consensus.toLocaleString()}
              </dd>
            </div>
          {/if}
        </dl>
        {#if seeksConsensus && config?.opening_racks}
          <p class="text-xs text-muted-foreground">
            Each rack is analysed {analysesPerRack(config.opening_racks)}; a rack is settled once
            they agree, or once it has been analysed the most times, and is not analysed again.
          </p>
        {/if}

        <div class="space-y-2 border-t border-border pt-4">
          <label class="label" for="rack">Look up a rack</label>
          <div class="flex gap-2">
            <input
              id="rack"
              class="input max-w-xs"
              bind:value={rackQuery}
              placeholder="AABCELT"
              on:keydown={(e) => e.key === 'Enter' && lookupRack()}
            />
            <button class="btn-primary" on:click={lookupRack}>Search</button>
          </div>
          {#if sampleRacks.length}
            <div class="flex flex-wrap items-center gap-2 text-sm">
              <span class="text-muted-foreground">Analysed racks to try:</span>
              {#each sampleRacks as rack}
                <button
                  class="rounded border border-border px-2 py-0.5 font-mono text-xs hover:bg-muted"
                  on:click={() => {
                    rackQuery = rack;
                    lookupRack();
                  }}>{rack}</button
                >
              {/each}
            </div>
          {/if}
          {#if rackError}<p class="field-error">{rackError}</p>{/if}
          {#if lookupConsensus && lookupConsensus.analyses > 1}
            <p class="text-sm" data-testid="rack-consensus">
              <span class="font-mono">{lookupConsensus.top}</span> is the best move in
              {lookupConsensus.count} of {lookupConsensus.analyses} analyses ({lookupConsensus.share}%).
            </p>
          {/if}
          {#if rackMoves?.length}
            <div class="overflow-x-auto">
            <table class="table whitespace-nowrap">
              <thead>
                <tr>
                  {#if lookupConsensus && lookupConsensus.analyses > 1}<th>Analysis</th>{/if}
                  <th>#</th><th>Move</th><th class="text-right">Score</th><th class="text-right">Equity</th>
                  {#if lookupWinPct}<th class="text-right">Win %</th>{/if}
                  {#if lookupIters}
                    <th class="text-right" title="How often the simulation played the move out">Iters</th>
                  {/if}
                  {#each plyHeaders(lookupPlies) as header}
                    <th class="text-right" title={header.title}>{header.label}</th>
                  {/each}
                </tr>
              </thead>
              <tbody>
                {#each rackMoves as move}
                  <tr class:border-t-2={lookupConsensus && lookupConsensus.analyses > 1 && move.rank === 1}>
                    {#if lookupConsensus && lookupConsensus.analyses > 1}
                      <td class="tabular-nums">{move.rank === 1 ? move.analysis : ''}</td>
                    {/if}
                    <td class="tabular-nums">{move.rank}</td>
                    <td class="font-mono text-xs">{move.move}</td>
                    <td class="text-right tabular-nums">{move.score}</td>
                    <td class="text-right tabular-nums">{move.equity.toFixed(2)}</td>
                    {#if lookupWinPct}
                      <td class="text-right tabular-nums">
                        {move.win_percentage === null ? '—' : move.win_percentage.toFixed(1)}
                      </td>
                    {/if}
                    {#if lookupIters}
                      <td class="text-right tabular-nums">
                        {move.iterations ? move.iterations.toLocaleString() : '—'}
                      </td>
                    {/if}
                    {#each Array(lookupPlies) as _, i}
                      {@const stats = plyAt(move.plies, i)}
                      <td class="text-right tabular-nums">{stats ? stats.average_score.toFixed(1) : '—'}</td>
                      <td class="text-right tabular-nums">
                        {stats ? `${stats.bingo_percentage.toFixed(1)}%` : '—'}
                      </td>
                    {/each}
                  </tr>
                {/each}
              </tbody>
            </table>
            </div>
          {/if}
        </div>
      </div>
    {/if}

    {#if stats.leave_generation}
      {@const lg = stats.leave_generation}
      <div class="card space-y-4">
        <h2 class="text-lg font-medium">
          Generation {lg.current_generation} of {lg.generation_count} — target
          {lg.target_rack_count.toLocaleString()} occurrences per rack
        </h2>
        {#if lg.generation_count > 1}
          <div class="overflow-x-auto">
            <table class="table text-xs" data-testid="leave-generations">
              <thead>
                <tr>
                  <th>Generation</th>
                  <th class="text-right">Target per rack</th>
                  <th>State</th>
                </tr>
              </thead>
              <tbody>
                {#each lg.target_rack_counts as target, i}
                  {@const generation = i + 1}
                  <tr class:font-medium={generation === lg.current_generation && generation > lg.generations_closed}>
                    <td class="tabular-nums">{generation}</td>
                    <td class="text-right tabular-nums">{target.toLocaleString()}</td>
                    <td>
                      {#if generation <= lg.generations_closed}closed{:else if generation === lg.current_generation}playing now{:else}to come{/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
        <p class="text-sm">
          <span class="tabular-nums">{lg.tasks_completed.toLocaleString()}</span> tasks and
          <span class="tabular-nums">{lg.games_played.toLocaleString()}</span> games played this
          generation{#if stats.job.status === 'active'}
            <span class="text-muted-foreground">— live.</span>{:else}.{/if}
        </p>
        <ProgressBar
          value={lg.racks_at_target}
          max={lg.racks_total}
          label="racks at target this generation"
        />
        <p class="text-sm text-muted-foreground">
          Fewest occurrences so far:
          <span class="font-mono text-foreground">{lg.min_rack ?? '—'}</span>
          at <span class="tabular-nums text-foreground">{lg.min_rack_count?.toLocaleString() ?? '—'}</span>.
          Rack figures are
          {#if lg.progress_as_of}
            as of {new Date(lg.progress_as_of).toLocaleTimeString()}:
          {:else}
            not computed yet:
          {/if}
          accepted results are merged into the per-rack totals in batches — every half hour, and
          about once a minute as a generation nears its end.
        </p>
      </div>
    {/if}

    <div class="card">
      <h2 class="mb-3 text-lg font-medium">Contributors</h2>
      <WorkerTable workers={stats.workers} />
      {#if stats.other_workers > 0}
        <p class="mt-2 text-sm text-muted-foreground">
          and {stats.other_workers.toLocaleString()} more
        </p>
      {/if}
    </div>

    <p class="text-sm text-muted-foreground">
      Raw results: <a href="/api/jobs/{jobId}/results">paginated JSON</a>. Bulk
      downloads are an admin operation — a full scan holds a database connection
      for as long as it runs.
    </p>
  </div>
{/if}
