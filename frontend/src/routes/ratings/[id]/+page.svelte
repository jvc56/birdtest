<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import {
    api,
    type PlayerConfig,
    type RatingHistoryPoint,
    type RatingPoolDetail
  } from '$lib/api';
  import { session } from '$lib/auth';
  import RatingDotPlot from '$lib/components/RatingDotPlot.svelte';
  import RatingHistoryChart from '$lib/components/RatingHistoryChart.svelte';
  import ResidualMatrix from '$lib/components/ResidualMatrix.svelte';
  import { ratingCell, stderrCell } from '$lib/charts/ratingDotPlot';

  let pool: RatingPoolDetail | null = null;
  let history: RatingHistoryPoint[] = [];
  let configs: PlayerConfig[] = [];
  let error = '';
  let loadError = '';
  let busy = false;
  let addConfigId = '';

  $: poolId = $page.params.id as string;
  $: isAdmin = $session?.is_admin ?? false;
  $: candidates = configs.filter(
    (c) => !pool?.ratings.some((r) => r.player_config_id === c.id)
  );

  // Both before either is shown: assigned one at a time, a failed history
  // left the pool on screen with an empty chart and the error nowhere.
  async function load() {
    const [loadedPool, loadedHistory] = await Promise.all([
      api.ratingPool(poolId),
      api.ratingHistory(poolId)
    ]);
    pool = loadedPool;
    history = loadedHistory;
  }

  onMount(async () => {
    try {
      await load();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    }
  });

  // Reactive on the session rather than read once in `load`: the layout asks
  // who is signed in at the same time this page loads, and on a hard refresh
  // its answer can arrive after the pool's -- when an admin was shown the
  // membership controls with nothing to add.
  let configsLoaded = false;
  $: if (isAdmin && !configsLoaded) {
    configsLoaded = true;
    api
      .playerConfigs()
      .then((list) => (configs = list))
      .catch(() => (configs = []));
  }

  /** Membership changes refit the whole pool, so the page reloads everything
   *  rather than patching one row: every other rating has moved too. */
  async function mutate(action: () => Promise<unknown>) {
    busy = true;
    error = '';
    try {
      await action();
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if pool}
  <section class="space-y-6">
    <div class="space-y-2">
      <h1 class="text-2xl font-semibold">{pool.name}</h1>
      <p class="text-sm text-muted-foreground">
        {pool.variant} · {pool.letter_distribution} · {pool.layout} — anchored at
        {pool.anchor_rating.toFixed(0)}
      </p>
      {#if pool.run}
        <p class="text-xs text-muted-foreground">
          Last fit {new Date(pool.run.computed_at).toLocaleString()} ({pool.run.trigger}) over
          {pool.run.pairs_used.toLocaleString()} pairs from {pool.run.jobs_used} job{pool.run
            .jobs_used === 1
            ? ''
            : 's'}, {pool.run.iterations} iterations.
        </p>
        {#if !pool.run.converged}
          <p class="text-xs text-warning">
            This fit did not converge. The ratings below are the last iterate, not a solution.
          </p>
        {/if}
      {:else}
        <p class="text-xs text-muted-foreground">Never computed.</p>
      {/if}
    </div>

    {#if error}
      <p class="text-sm text-destructive">{error}</p>
    {/if}

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">Ratings</h2>
      <RatingDotPlot ratings={pool.ratings} />
    </div>

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">All configs</h2>
      <table class="table">
        <thead>
          <tr>
            <th>Player config</th>
            <th class="text-right">Rating</th>
            <th class="text-right">± SE</th>
            <th class="text-right">Pairs</th>
            {#if isAdmin}<th></th>{/if}
          </tr>
        </thead>
        <tbody>
          {#each pool.ratings as row}
            <tr>
              <td>
                {row.name}
                {#if row.is_anchor}
                  <span class="ml-1 text-xs text-warning">anchor</span>
                {/if}
              </td>
              <td class="text-right tabular-nums">{ratingCell(row)}</td>
              <td class="text-right tabular-nums text-muted-foreground">{stderrCell(row)}</td>
              <td class="text-right tabular-nums">{row.pairs_played.toLocaleString()}</td>
              {#if isAdmin}
                <td class="text-right">
                  {#if !row.is_anchor}
                    <button
                      class="btn-secondary text-xs"
                      disabled={busy}
                      on:click={() =>
                        mutate(() => api.removeRatingPoolMember(poolId, row.player_config_id))}
                    >
                      Remove
                    </button>
                  {/if}
                </td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>

      {#if isAdmin}
        <div class="flex flex-wrap items-end gap-2 border-t border-border pt-3">
          <div class="min-w-56 flex-1">
            <label class="label" for="add-config">Add a player config</label>
            <select id="add-config" class="input" bind:value={addConfigId} disabled={busy}>
              <option value="">Select…</option>
              {#each candidates as config}
                <option value={config.id}>{config.name}</option>
              {/each}
            </select>
          </div>
          <button
            class="btn-primary"
            disabled={busy || !addConfigId}
            on:click={() =>
              mutate(async () => {
                await api.addRatingPoolMember(poolId, addConfigId);
                addConfigId = '';
              })}
          >
            Add
          </button>
          <button
            class="btn-secondary"
            disabled={busy}
            on:click={() => mutate(() => api.recomputeRatingPool(poolId))}
          >
            Recompute
          </button>
        </div>
        <p class="text-xs text-muted-foreground">
          Adding or removing a config refits the whole pool. A config's games are evidence for
          everyone else's rating too, so taking it out moves every other number — that is correct,
          and it is why this is not a per-row edit.
        </p>
      {/if}
    </div>

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">Rating history</h2>
      <RatingHistoryChart {history} />
    </div>

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">Where the model disagrees with the games</h2>
      <p class="text-sm text-muted-foreground">
        One rating per config cannot express a cycle — A beating B, B beating C and C beating A.
        These are the head-to-heads the fitted ratings predict worst, largest first.
      </p>
      <ResidualMatrix residuals={pool.residuals} ratings={pool.ratings} />
    </div>
  </section>
{:else if loadError}
  <p class="text-sm text-destructive">Could not load this rating pool: {loadError}</p>
{:else}
  <p class="text-sm text-muted-foreground">Loading…</p>
{/if}
