<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api, errorText, type PlayerConfig, type RatingPoolDetail } from '$lib/api';
  import { session } from '$lib/auth';
  import RatingDotPlot from '$lib/components/RatingDotPlot.svelte';
  import { ratingCell, stderrCell } from '$lib/charts/ratingDotPlot';
  import { fitLabels } from '$lib/charts/labels';
  import { isNonTransitive, NOTABLE, significantResiduals } from '$lib/charts/residuals';
  import {
    cellTitle,
    crossTable,
    oneSide,
    poolMembership,
    spreadText,
    winText
  } from '$lib/ratingPool';

  let pool: RatingPoolDetail | null = null;
  let configs: PlayerConfig[] = [];
  let error = '';
  /** What a change that went through but changed nothing says instead. */
  let notice = '';
  let loadError = '';
  let busy = false;
  let addConfigId = '';
  let anchorId = '';
  /** Null while the input is empty: a number input binds a number or nothing. */
  let anchorRating: number | null = null;

  $: poolId = $page.params.id as string;
  $: isAdmin = $session?.is_admin ?? false;
  // From the pool's membership, not the latest fit's ratings: a member added
  // since that fit, or whose refit failed, is listed as not yet rated, with a
  // Remove button, and is not offered under "Add" (the anchor of a pool never
  // fitted included).
  $: membership = pool ? poolMembership(pool, configs) : null;
  $: table = pool ? crossTable(pool) : null;
  $: columnLabels = table ? fitLabels(table.configs.map((c) => c.name)) : [];
  // Each head-to-head once: a cell and its mirror are the same miss.
  $: misses = pool ? significantResiduals(oneSide(pool.head_to_heads)) : [];
  $: nonTransitive = pool ? isNonTransitive(oneSide(pool.head_to_heads)) : false;
  /** A cell the ratings predict badly, on enough pairs that it is not chance. */
  $: missed = new Set(misses.flatMap((c) => [`${c.row}:${c.col}`, `${c.col}:${c.row}`]));
  $: members = pool?.members ?? [];
  $: others = membership?.others ?? [];
  // The anchor form starts from the pool as stored, and again after every
  // reload, so an unsaved edit never survives a change that moved the pool.
  $: anchorChanged =
    !!pool &&
    (anchorId !== pool.anchor_player_config_id || anchorRating !== pool.anchor_rating);

  async function load() {
    pool = await api.ratingPool(poolId);
    anchorId = pool.anchor_player_config_id;
    anchorRating = pool.anchor_rating;
  }

  async function saveAnchor() {
    if (!pool) return;
    const id = anchorId;
    const rating = anchorRating;
    // The server's bound (`MAX_ABS_ANCHOR_RATING`), checked here so the admin
    // is told before a request rather than after it.
    if (rating == null || !Number.isFinite(rating) || Math.abs(rating) > 10000) {
      error = 'The anchor rating must be a number between -10000 and 10000.';
      return;
    }
    const body: { anchor_player_config_id?: string; anchor_rating?: number } = {};
    if (id !== pool.anchor_player_config_id) body.anchor_player_config_id = id;
    if (rating !== pool.anchor_rating) body.anchor_rating = rating;
    // A refused change leaves what was typed in the form to correct, rather
    // than the reload's stored values.
    if (!(await mutate(() => api.updateRatingPool(poolId, body)))) {
      anchorId = id;
      anchorRating = rating;
    }
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
      error = errorText(e);
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
   *  rather than patching one row: every other rating has moved too. Says
   *  whether the change itself succeeded. */
  async function mutate(action: () => Promise<unknown>): Promise<boolean> {
    busy = true;
    error = '';
    notice = '';
    let ok = true;
    try {
      await action();
    } catch (e) {
      ok = false;
      error = errorText(e);
    }
    // Reloaded either way: a change whose refit failed has still committed.
    try {
      await load();
    } catch (e) {
      error ||= errorText(e);
    } finally {
      busy = false;
    }
    return ok;
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
    {:else if notice}
      <p class="text-sm text-muted-foreground">{notice}</p>
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
            <th class="text-right">Rating (WESPA scale)</th>
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
                  <!-- A config removed since the latest fit is still rated by it. -->
                  {#if !membership?.memberIds.has(row.player_config_id)}
                    <span class="text-xs text-muted-foreground">removed</span>
                  {:else if !row.is_anchor}
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
          {#each membership?.unrated ?? [] as member}
            <tr>
              <td>
                {member.name}
                {#if member.player_config_id === pool.anchor_player_config_id}
                  <span class="ml-1 text-xs text-warning">anchor</span>
                {/if}
              </td>
              <td class="text-right text-muted-foreground">not yet rated</td>
              <td class="text-right text-muted-foreground">—</td>
              <td class="text-right text-muted-foreground">—</td>
              {#if isAdmin}
                <td class="text-right">
                  {#if member.player_config_id !== pool.anchor_player_config_id}
                    <button
                      class="btn-secondary text-xs"
                      disabled={busy}
                      on:click={() =>
                        mutate(() => api.removeRatingPoolMember(poolId, member.player_config_id))}
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
      <p class="text-xs text-muted-foreground">
        A gap between two ratings predicts the score the same gap does between two established
        WESPA players; the absolute level is only where the anchor was pinned, since bot games say
        nothing about strength against people.
      </p>

      {#if isAdmin}
        <div class="flex flex-wrap items-end gap-2 border-t border-border pt-3">
          <div class="min-w-56 flex-1">
            <label class="label" for="add-config">Add a player config</label>
            <select id="add-config" class="input" bind:value={addConfigId} disabled={busy}>
              <option value="">Select…</option>
              {#each others as config}
                <option value={config.id}>{config.name}</option>
              {/each}
            </select>
          </div>
          <button
            class="btn-primary"
            disabled={busy || !addConfigId}
            on:click={() =>
              mutate(async () => {
                // Null for a config that is already a member: neither logged
                // nor refitted.
                const { run_id } = await api.addRatingPoolMember(poolId, addConfigId);
                addConfigId = '';
                if (run_id === null) {
                  notice = 'Already a member: nothing changed. Use Recompute to refit.';
                }
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
                  <option value={member.player_config_id}>{member.name}</option>
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
              min="-10000"
              max="10000"
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
      <h2 class="text-lg font-medium">Cross table</h2>
      {#if table && pool.head_to_heads.length}
        <p class="text-sm text-muted-foreground">
          Each cell is the row config's score against the column's — (wins + ½ draws) / games —
          ± its standard error, and beneath it the row's average spread per game. Hover a cell for
          what the ratings predict; amber is a score they predict badly, on enough pairs that it is
          not chance.
        </p>
        <!-- Scrolls inside the card on a phone, the row names held in place. -->
        <div class="overflow-x-auto">
          <table class="table text-xs" data-testid="cross-table">
            <thead>
              <tr>
                <th class="sticky left-0 z-10 bg-card">Player config</th>
                {#each table.configs as col, c}
                  <th class="whitespace-nowrap text-center" title={col.name}>{columnLabels[c]}</th>
                {/each}
                <th class="whitespace-nowrap text-right">Rating</th>
              </tr>
            </thead>
            <tbody>
              {#each table.configs as row}
                <tr>
                  <th scope="row" class="sticky left-0 z-10 bg-card">
                    <!-- Held to a third of a phone, so the cells have the rest. -->
                    <span class="block max-w-[8rem] truncate sm:max-w-xs" title={row.name}>
                      {row.name}
                    </span>
                  </th>
                  {#each table.configs as col}
                    {@const cell = table.cell(row.player_config_id, col.player_config_id)}
                    {#if row.player_config_id === col.player_config_id}
                      <td class="bg-muted/40"></td>
                    {:else if cell}
                      <td
                        class="whitespace-nowrap text-center tabular-nums"
                        class:text-warning={missed.has(`${cell.row}:${cell.col}`)}
                        title={cellTitle(cell, row.name, col.name)}
                      >
                        <div>{winText(cell)}</div>
                        <div class="text-muted-foreground">{spreadText(cell)}</div>
                      </td>
                    {:else}
                      <td class="text-center text-muted-foreground">·</td>
                    {/if}
                  {/each}
                  <td class="whitespace-nowrap text-right tabular-nums">{ratingCell(row)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if nonTransitive}
          <p class="text-xs text-warning">
            {misses.length} head-to-heads are more than {(100 * NOTABLE).toFixed(0)} points
            from what the ratings predict, on enough pairs that it is not chance. That is the signature
            of a non-transitive pool — configs that beat some opponents and lose to others in a way no
            single number per config can express. Read the ratings as a summary here, not as a
            ranking.
          </p>
        {/if}
      {:else}
        <p class="text-sm text-muted-foreground">No head-to-head results in this pool yet.</p>
      {/if}
    </div>
  </section>
{:else if loadError}
  <p class="text-sm text-destructive">Could not load this rating pool: {loadError}</p>
{:else}
  <p class="text-sm text-muted-foreground">Loading…</p>
{/if}
