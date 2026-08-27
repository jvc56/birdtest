<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { api, type JobStats } from '$lib/api';
  import { subscribeToJob } from '$lib/sse';
  import { duration, datetime, jobTypeLabel, sprtLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';
  import WorkerTable from '$lib/components/WorkerTable.svelte';
  import ProgressBar from '$lib/components/ProgressBar.svelte';
  import OutcomeChart from '$lib/components/OutcomeChart.svelte';

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
    </header>

    <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
      <div class="card">
        <p class="text-xs uppercase text-muted-foreground">Priority / allocation</p>
        <p class="mt-1 text-xl tabular-nums">
          {stats.job.priority} · {stats.job.allocation === null ? '—' : `${stats.job.allocation}%`}
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
          <JobStatusBadge status={stats.games.sprt.status} />
        </div>
        <p class="text-sm text-muted-foreground">
          {sprtLabel(stats.games.sprt.status)} — LLR {stats.games.sprt.llr.toFixed(3)} within
          [{stats.games.sprt.lower_bound.toFixed(2)}, {stats.games.sprt.upper_bound.toFixed(2)}].
          SPRT is not acted on until {stats.games.min_units.toLocaleString()}
          {stats.games.unit}s are complete.
        </p>
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
        {#if stats.games.divergent_pairs !== undefined}
          <p class="text-xs text-muted-foreground">
            Counted over {stats.games.divergent_pairs.toLocaleString()} divergent pairs of
            {stats.games.units_completed.toLocaleString()} played. Pairs whose two games play
            identically are guaranteed ties and carry no signal, so they are excluded.
          </p>
        {/if}
      </div>
    {/if}

    {#if stats.ratings.length}
      <div class="card">
        <h2 class="mb-3 text-lg font-medium">Glicko snapshot</h2>
        <table class="table">
          <thead>
            <tr><th>Player config</th><th class="text-right">Rating</th><th class="text-right">RD</th><th class="text-right">Pairs</th></tr>
          </thead>
          <tbody>
            {#each stats.ratings as rating}
              <tr>
                <td>{rating.name}</td>
                <td class="text-right tabular-nums">{rating.rating.toFixed(1)}</td>
                <td class="text-right tabular-nums">±{rating.rating_deviation.toFixed(1)}</td>
                <td class="text-right tabular-nums">{rating.games_played.toLocaleString()}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if stats.opening_racks}
      <div class="card space-y-4">
        <h2 class="text-lg font-medium">Opening racks</h2>
        <dl class="grid grid-cols-2 gap-4 text-sm sm:grid-cols-3">
          <div>
            <dt class="text-muted-foreground">Racks analyzed</dt>
            <dd class="text-xl tabular-nums">{stats.opening_racks.racks_analyzed.toLocaleString()}</dd>
          </div>
          <div>
            <dt class="text-muted-foreground">Average best equity</dt>
            <dd class="text-xl tabular-nums">
              {stats.opening_racks.average_best_equity?.toFixed(2) ?? '—'}
            </dd>
          </div>
          <div>
            <dt class="text-muted-foreground">Best move types</dt>
            <dd class="text-sm">
              {#each stats.opening_racks.best_move_types as type, i}
                {i ? ' · ' : ''}{type.move_type} {type.count.toLocaleString()}
              {:else}—{/each}
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
        <ProgressBar
          value={lg.racks_at_target}
          max={lg.racks_total}
          label="racks at target this generation"
        />
        <p class="text-sm text-muted-foreground">
          Fewest occurrences so far:
          <span class="font-mono text-foreground">{lg.min_rack ?? '—'}</span>
          at <span class="tabular-nums text-foreground">{lg.min_rack_count?.toLocaleString() ?? '—'}</span>.
          This updates on every accepted result, not on a worker heartbeat.
        </p>
      </div>
    {/if}

    <div class="card">
      <h2 class="mb-3 text-lg font-medium">Contributors</h2>
      <WorkerTable workers={stats.workers} />
    </div>

    <p class="text-sm text-muted-foreground">
      Raw results: <a href="/api/jobs/{jobId}/results">paginated JSON</a> ·
      <a href="/api/jobs/{jobId}/results/stream">full NDJSON download</a>
    </p>
  </div>
{/if}
