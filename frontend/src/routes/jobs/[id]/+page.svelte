<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { api, type JobStats } from '$lib/api';
  import { subscribeToJob } from '$lib/sse';
  import { session } from '$lib/auth';
  import { duration, datetime, jobTypeLabel, sprtLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';
  import WorkerTable from '$lib/components/WorkerTable.svelte';
  import ProgressBar from '$lib/components/ProgressBar.svelte';
  import OutcomeChart from '$lib/components/OutcomeChart.svelte';
  import { pentanomialRows } from '$lib/charts/pentanomial';

  // The [id] route only matches when the param is present.
  const jobId = $page.params.id as string;

  let stats: JobStats | null = null;
  let error = '';

  // Opening-rack search
  let rackQuery = '';
  let rackMoves: Record<string, unknown>[] | null = null;
  let rackError = '';

  onMount(() => {
    api
      .job(jobId)
      .then((value) => (stats = value))
      .catch((e) => (error = e.message));
    // The stream carries the same payload as the REST call, so an update is a
    // straight replacement rather than a merge.
    return subscribeToJob<JobStats>(jobId, (value) => (stats = value));
  });

  async function lookupRack() {
    rackError = '';
    rackMoves = null;
    try {
      const result = await api.jobResults(jobId, { rack: rackQuery });
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
      <h1 class="text-2xl font-semibold">{jobTypeLabel(stats.job.job_type)}</h1>
      <JobStatusBadge status={stats.job.status} />
      <span class="text-sm text-muted-foreground">
        {stats.job.lexicon ?? '—'} · {stats.job.variant ?? '—'}
      </span>
      <!-- The admin page (activate, purge, export, artifacts) was reachable
           only by the redirect after creating the job. -->
      {#if $session?.is_admin}
        <a href="/admin/jobs/{stats.job.id}" class="btn-secondary ml-auto no-underline">Manage</a>
      {/if}
    </header>

    <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <div class="card">
        <p class="text-xs uppercase text-muted-foreground">Allocation</p>
        <p class="mt-1 text-xl tabular-nums">
          {stats.job.allocation === null ? '—' : `${stats.job.allocation}%`}
        </p>
      </div>
      <div class="card">
        <p class="text-xs uppercase text-muted-foreground">Redundancy</p>
        <p class="mt-1 text-xl tabular-nums">{stats.job.redundancy}×</p>
      </div>
      <div class="card">
        <p class="text-xs uppercase text-muted-foreground">Results accepted</p>
        <p class="mt-1 text-xl tabular-nums">{stats.results_accepted.toLocaleString()}</p>
      </div>
      <div class="card">
        <p class="text-xs uppercase text-muted-foreground">Estimated time left</p>
        <p class="mt-1 text-xl tabular-nums">{duration(stats.eta_seconds)}</p>
      </div>
    </div>

    <div class="card space-y-4">
      <h2 class="text-lg font-medium">Progress</h2>
      {#if stats.games}
        <ProgressBar
          value={stats.games.units_completed}
          max={stats.games.max_units}
          label="{stats.games.unit}s completed (hard cap)"
        />
      {:else if stats.opening_racks}
        <!-- Tasks are made on demand, so a task count is only what has been
             handed out so far: a job 1% through its racks read 99%. -->
        <ProgressBar
          value={stats.opening_racks.racks_analyzed}
          max={stats.opening_racks.racks_total}
          label="racks analysed"
        />
      {:else if stats.leave_generation}
        <ProgressBar
          value={Math.max(stats.leave_generation.current_generation - 1, 0)}
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
      <dl class="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-4">
        <div><dt class="text-muted-foreground">Available</dt><dd class="tabular-nums">{stats.tasks_available.toLocaleString()}</dd></div>
        <div><dt class="text-muted-foreground">Claimed</dt><dd class="tabular-nums">{stats.tasks_claimed.toLocaleString()}</dd></div>
        <div><dt class="text-muted-foreground">Completed</dt><dd class="tabular-nums">{stats.tasks_completed.toLocaleString()}</dd></div>
        <div><dt class="text-muted-foreground">Created</dt><dd>{datetime(stats.job.created_at)}</dd></div>
      </dl>
      <p class="text-xs text-muted-foreground">
        Created by {stats.job.created_by ?? 'unknown'}{#if stats.job.min_magpie_version}
          · requires MAGPIE ≥ {stats.job.min_magpie_version}{/if}
      </p>
    </div>

    {#if stats.games}
      <div class="card space-y-4">
        <div class="flex items-center justify-between">
          <h2 class="text-lg font-medium">SPRT</h2>
          <JobStatusBadge status={stats.games.decided?.status ?? stats.games.sprt.status} />
        </div>
        {#if stats.games.decided}
          <p class="text-sm text-muted-foreground">
            Completed: {sprtLabel(stats.games.decided.status)} at LLR
            {stats.games.decided.llr.toFixed(3)} after {stats.games.decided.units.toLocaleString()}
            {stats.games.unit}s. With the {stats.games.unit}s that were in flight then, LLR
            {stats.games.sprt.llr.toFixed(3)} within [{stats.games.sprt.lower_bound.toFixed(2)},
            {stats.games.sprt.upper_bound.toFixed(2)}].
          </p>
        {:else}
          <p class="text-sm text-muted-foreground">
            {sprtLabel(stats.games.sprt.status)} — LLR {stats.games.sprt.llr.toFixed(3)} within
            [{stats.games.sprt.lower_bound.toFixed(2)}, {stats.games.sprt.upper_bound.toFixed(2)}].
            SPRT is not acted on until {stats.games.min_units.toLocaleString()}
            {stats.games.unit}s are complete.
          </p>
        {/if}
        <OutcomeChart
          wins={stats.games.wins}
          losses={stats.games.losses}
          draws={stats.games.draws}
        />
        <p class="text-sm tabular-nums text-muted-foreground">
          Player 1: {stats.games.wins.toLocaleString()} W ({stats.games.win_pct.toFixed(1)}%) ·
          {stats.games.losses.toLocaleString()} L ({stats.games.loss_pct.toFixed(1)}%) ·
          {stats.games.draws.toLocaleString()} D ({stats.games.draw_pct.toFixed(1)}%)
        </p>
        {#if stats.games.pentanomial}
          <div class="space-y-1">
            <p class="text-xs text-muted-foreground">
              The test runs on all {stats.games.units_completed.toLocaleString()} pairs, scored by
              player 1's result across the pair. Pairs whose two games played identically are 1-1
              ties — they stay in the sample, where they are what makes a paired run
              lower-variance than an unpaired one.
            </p>
            <table class="table text-xs">
              <thead>
                <tr>
                  <th>Pair outcome</th>
                  <th class="text-right">Pairs</th>
                  <th class="text-right">Share</th>
                </tr>
              </thead>
              <tbody>
                {#each pentanomialRows(stats.games.pentanomial, stats.games.units_completed) as bucket}
                  <tr>
                    <td>{bucket.label}</td>
                    <td class="text-right tabular-nums">{bucket.pairs.toLocaleString()}</td>
                    <td class="text-right tabular-nums">{bucket.share}%</td>
                  </tr>
                {/each}
              </tbody>
            </table>
            {#if stats.games.divergent_pairs !== undefined}
              <p class="text-xs text-muted-foreground">
                {stats.games.divergent_pairs.toLocaleString()} of {stats.games.units_completed.toLocaleString()}
                pairs diverged — a diagnostic of how often these two configs differ at all, not
                part of the test.
              </p>
            {/if}
          </div>
        {/if}
      </div>
    {/if}

    <!-- Ratings are pool-scoped and live on /ratings: a rating is a statement
         about a player config across every pair it has played, not something
         one job owns. -->

    {#if stats.opening_racks}
      <div class="card space-y-4">
        <h2 class="text-lg font-medium">Opening racks</h2>
        <dl class="grid grid-cols-2 gap-4 text-sm">
          <div>
            <dt class="text-muted-foreground">Racks analyzed</dt>
            <dd class="text-xl tabular-nums">
              {stats.opening_racks.racks_analyzed.toLocaleString()}
              <span class="text-sm text-muted-foreground">
                / {stats.opening_racks.racks_total.toLocaleString()}
              </span>
            </dd>
          </div>
        </dl>

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
          {#if rackError}<p class="field-error">{rackError}</p>{/if}
          {#if rackMoves?.length}
            <table class="table">
              <thead>
                <tr><th>#</th><th>Move</th><th class="text-right">Score</th><th class="text-right">Equity</th></tr>
              </thead>
              <tbody>
                {#each rackMoves as move}
                  <tr>
                    <td class="tabular-nums">{move.rank}</td>
                    <td class="font-mono text-xs">{move.move}</td>
                    <td class="text-right tabular-nums">{move.score}</td>
                    <td class="text-right tabular-nums">{Number(move.equity).toFixed(2)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
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
        <p class="text-sm">
          <span class="tabular-nums">{lg.tasks_completed.toLocaleString()}</span> tasks and
          <span class="tabular-nums">{lg.games_played.toLocaleString()}</span> games played this
          generation
          <span class="text-muted-foreground">— live, on every accepted result.</span>
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
