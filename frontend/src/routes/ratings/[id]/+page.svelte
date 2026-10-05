<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api, type PlayerConfig, type RatingPoolDetail } from '$lib/api';
  import { session } from '$lib/auth';
  import RatingDotPlot from '$lib/components/RatingDotPlot.svelte';
  import ResidualMatrix from '$lib/components/ResidualMatrix.svelte';
  import { ratingCell, stderrCell } from '$lib/charts/ratingDotPlot';

  let pool: RatingPoolDetail | null = null;
  let configs: PlayerConfig[] = [];
  let error = '';
  let loadError = '';
  let busy = false;
  let addConfigId = '';
  let anchorId = '';
  /** Null while the input is empty: a number input binds a number or nothing. */
  let anchorRating: number | null = null;

  $: poolId = $page.params.id as string;
  $: isAdmin = $session?.is_admin ?? false;
  $: candidates = configs.filter(
    (c) => !pool?.ratings.some((r) => r.player_config_id === c.id)
  );

  // The anchor form starts from the pool as stored, and again after every
  // reload, so an unsaved edit never survives a change that moved the pool.
  // Members as the latest fit rated them -- plus the anchor itself, which a
  // pool that has never been fitted has rated no one, itself included.
  $: members = pool ? poolMembers(pool, configs) : [];
  $: others = configs.filter((c) => !members.some((m) => m.id === c.id));
  $: anchorChanged =
    !!pool &&
    (anchorId !== pool.anchor_player_config_id || anchorRating !== pool.anchor_rating);

  function poolMembers(pool: RatingPoolDetail, configs: PlayerConfig[]) {
    const rated = pool.ratings.map((r) => ({ id: r.player_config_id, name: r.name }));
    if (rated.some((m) => m.id === pool.anchor_player_config_id)) return rated;
    const anchor = configs.find((c) => c.id === pool.anchor_player_config_id);
    return [{ id: pool.anchor_player_config_id, name: anchor?.name ?? 'the current anchor' }, ...rated];
  }

  async function load() {
    pool = await api.ratingPool(poolId);
    anchorId = pool.anchor_player_config_id;
    anchorRating = pool.anchor_rating;
  }

  function saveAnchor() {
    if (!pool) return;
    const rating = anchorRating;
    if (rating == null || !Number.isFinite(rating)) {
      error = 'The anchor rating must be a number.';
      return;
    }
    const body: { anchor_player_config_id?: string; anchor_rating?: number } = {};
    if (anchorId !== pool.anchor_player_config_id) body.anchor_player_config_id = anchorId;
    if (rating !== pool.anchor_rating) body.anchor_rating = rating;
    mutate(() => api.updateRatingPool(poolId, body));
  }

  async function removePool() {
    if (!pool || busy) return;
    if (
      !confirm(
        `Delete the rating pool "${pool.name}" and its whole rating history? ` +
          'The games stay with their jobs. This cannot be undone.'
      )
    )
      return;
    busy = true;
    error = '';
    try {
      await api.deleteRatingPool(poolId);
      goto('/ratings');
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      busy = false;
    }
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
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    // Reloaded either way: a change whose refit failed has still committed.
    try {
      await load();
    } catch (e) {
      error ||= e instanceof Error ? e.message : String(e);
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
      <div class="overflow-x-auto">
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
      </div>

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

    {#if isAdmin}
      <div class="card space-y-3">
        <h2 class="text-lg font-medium">Anchor</h2>
        <div class="flex flex-wrap items-end gap-2">
          <div class="min-w-56 flex-1">
            <label class="label" for="anchor-config">Anchor player config</label>
            <select id="anchor-config" class="input" bind:value={anchorId} disabled={busy}>
              <optgroup label="Members">
                {#each members as member}
                  <option value={member.id}>{member.name}</option>
                {/each}
              </optgroup>
              {#if others.length}
                <optgroup label="Other player configs (joins the pool)">
                  {#each others as config}
                    <option value={config.id}>{config.name}</option>
                  {/each}
                </optgroup>
              {/if}
            </select>
          </div>
          <div class="w-36">
            <label class="label" for="anchor-rating">Anchor rating</label>
            <input
              id="anchor-rating"
              class="input"
              type="number"
              step="any"
              bind:value={anchorRating}
              disabled={busy}
            />
          </div>
          <button class="btn-primary" disabled={busy || !anchorChanged} on:click={saveAnchor}>
            Save
          </button>
        </div>
        <p class="text-xs text-muted-foreground">
          Every rating is measured from the anchor, so moving it or its rating refits the pool on
          the new scale. Earlier runs keep the scale they were fitted on. A config that is not a
          member joins the pool as its anchor.
        </p>
        <div class="flex flex-wrap items-center gap-3 border-t border-border pt-3">
          <button class="btn-destructive" disabled={busy} on:click={removePool}>
            Delete pool
          </button>
          <p class="text-xs text-muted-foreground">
            Deletes the pool and its rating history. The games stay with their jobs.
          </p>
        </div>
      </div>
    {/if}

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
