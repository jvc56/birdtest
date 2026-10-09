<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    api,
    type Contributor,
    type ContributorSort,
    type MovegensByType,
    type Page
  } from '$lib/api';
  import { computeTime, datetime, exactCount, workerLabel } from '$lib/format';
  import { contributorKey, movegensLines } from '$lib/movegens';
  import { createPoller } from '$lib/poller';
  import Pagination from '$lib/components/Pagination.svelte';

  /** How often the list is read again while the page is shown. */
  const REFRESH_MS = 30_000;

  let result: Page<Contributor> | null = null;
  let loadError = '';
  // Movegens by default: the work a machine actually did, as MAGPIE counts it.
  // Tasks undercount one that plays deep, slow games, and compute time -- how
  // long its claims were held -- overcounts a slow machine.
  let sort: ContributorSort = 'movegens';
  // The page asked for, which each refresh reads again.
  let current = 0;
  // Each load is numbered, and only the newest one's answer is shown: a
  // header clicked while the previous order is still loading must not be
  // overwritten by it -- nor by a refresh of the old order still in flight.
  let generation = 0;
  // The site's movegens by job type, read with each page of the list. Null
  // until the first answer; a refresh that fails keeps the last.
  let siteTotals: MovegensByType | null = null;

  /** One contributor's breakdown by job type, open under their row. */
  interface Breakdown {
    value: MovegensByType | null;
    error: string;
    /** The newest read of it, so an older answer arriving late is dropped. */
    asked: number;
  }
  // Open breakdowns by `contributorKey`. Kept across refreshes, re-sorts and
  // pages, so a refresh redraws the list without folding anything up: a row
  // open when the list is read again is still open, and read again with it.
  let open: Record<string, Breakdown> = {};
  let asks = 0;

  async function readBreakdown(key: string) {
    const asked = ++asks;
    open[key] = { ...(open[key] ?? { value: null, error: '' }), asked };
    try {
      const value = await api.contributorMovegens(key);
      if (open[key]?.asked === asked) open[key] = { value, error: '', asked };
    } catch (e) {
      // Shown in the row; an earlier answer stays beside it.
      const error = e instanceof Error ? e.message : String(e);
      if (open[key]?.asked === asked) open[key] = { ...open[key], error };
    }
  }

  function toggle(key: string) {
    if (open[key]) {
      delete open[key];
      open = open;
    } else {
      readBreakdown(key);
    }
  }

  const columns: { sort: ContributorSort; label: string }[] = [
    { sort: 'movegens', label: 'Movegens' },
    { sort: 'compute', label: 'Compute time' },
    { sort: 'tasks', label: 'Tasks' }
  ];

  async function load() {
    const mine = ++generation;
    // Beside the list rather than with it: neither waits on the other, and
    // the totals failing leaves the list to refresh.
    const totals = api.siteMovegens().then(
      (value) => {
        if (mine === generation) siteTotals = value;
      },
      () => {}
    );
    try {
      const next = await api.workers(current, sort);
      if (mine === generation) {
        result = next;
        loadError = '';
        // The open rows on this page, read again with it: their figures
        // would otherwise fall behind the row's own as contributors work.
        for (const worker of next.items) {
          const key = contributorKey(worker);
          if (key && open[key]) readBreakdown(key);
        }
      }
    } catch (e) {
      if (mine === generation) loadError = e instanceof Error ? e.message : String(e);
    }
    await totals;
  }

  // Read again every half minute while the tab is shown, and at once when it
  // is shown again: the counts rise as contributors work. `load` reports its
  // own answers, so the poller only keeps time.
  const poller = createPoller(
    { read: load, onValue: () => {}, onError: () => {}, delay: () => REFRESH_MS },
    document
  );

  /** Shows `page` now; the next refresh is a full interval after it. */
  function show(page: number) {
    current = page;
    poller.refresh();
  }

  function rankBy(next: ContributorSort) {
    if (next === sort) return;
    sort = next;
    show(0);
  }

  function value(worker: Contributor, column: ContributorSort): string {
    switch (column) {
      case 'movegens':
        return exactCount(worker.movegens);
      case 'compute':
        return computeTime(worker.compute_seconds);
      case 'tasks':
        return worker.tasks_completed.toLocaleString();
    }
  }

  // On a phone only the column the list is ranked by is shown beside the
  // name: with a pseudonym's sixteen characters, any second one pushed it out
  // of the box.
  $: secondary = (column: ContributorSort) => (column === sort ? '' : 'hidden sm:table-cell');

  onMount(() => poller.start());
  onDestroy(() => poller.stop());
</script>

<h1 class="mb-2 text-2xl font-semibold">Contributors</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Every worker that has completed a task, authenticated or anonymous, ranked by movegens: the move
  generations MAGPIE performed for it, summed over every task it finished. Compute time is how long
  its claims were held, from claim to result. Choose a column to rank by it instead, or a
  contributor to see their movegens by job type. The list refreshes itself every 30 seconds.
</p>

{#if siteTotals}
  <!-- Every contributor's movegens together, by the type of job they were for:
       the jobs' own totals, which a purged or deleted job leaves as it leaves
       the list. One to a line on a phone, so a count to its last digit never
       runs off the side. -->
  <section class="card mb-6" aria-labelledby="site-movegens">
    <h2 id="site-movegens" class="mb-3 text-sm font-medium">Movegens by job type</h2>
    <dl class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4" data-testid="site-movegens">
      {#each movegensLines(siteTotals) as line}
        <div class="min-w-0">
          <dt class="text-xs uppercase text-muted-foreground">{line.label}</dt>
          <dd class="mt-1 break-all text-lg tabular-nums">{exactCount(line.movegens)}</dd>
        </div>
      {/each}
    </dl>
  </section>
{/if}

{#if loadError}
  <!-- Above the list rather than instead of it: a refresh that fails leaves
       the last answer on screen. -->
  <p class="mb-3 text-sm text-destructive">Could not load the contributors: {loadError}</p>
{/if}
{#if !result}
  {#if !loadError}<p class="text-muted-foreground">Loading…</p>{/if}
{:else}
  <!-- A phone shows only the ranked column, so its header is the only one on
       screen to choose from: the choice is made here instead. -->
  <div class="mb-3 flex flex-wrap items-center gap-2 text-sm sm:hidden">
    <span class="text-muted-foreground">Rank by</span>
    {#each columns as column}
      <button
        class={column.sort === sort ? 'btn-primary' : 'btn-secondary'}
        aria-pressed={column.sort === sort}
        on:click={() => rankBy(column.sort)}>{column.label}</button
      >
    {/each}
  </div>
  <!-- Tighter cells on a phone: a count to its last digit is up to 25
       characters (a BIGINT's), and at the site's padding the widest one with
       its name and rank was a pixel wider than a Pixel 5's box. -->
  <div class="card overflow-x-auto p-0">
    <table class="table [&_td]:px-2 [&_th]:px-2 sm:[&_td]:px-3 sm:[&_th]:px-3">
      <thead>
        <tr>
          <th>#</th>
          <th>Contributor</th>
          {#each columns as column}
            <th
              class="text-right {secondary(column.sort)}"
              aria-sort={column.sort === sort ? 'descending' : 'none'}
            >
              <button
                class="inline-flex items-center gap-1 font-medium hover:text-foreground {column.sort ===
                sort
                  ? 'text-foreground'
                  : ''}"
                on:click={() => rankBy(column.sort)}
              >
                {column.label}<span aria-hidden="true" class:invisible={column.sort !== sort}>↓</span>
              </button>
            </th>
          {/each}
          <th class="hidden sm:table-cell">Last result</th>
        </tr>
      </thead>
      <tbody>
        {#each result.items as worker, i}
          {@const key = contributorKey(worker)}
          {@const breakdown = key ? open[key] : undefined}
          <tr>
            <td class="tabular-nums text-muted-foreground">{result.page * result.per_page + i + 1}</td>
            <td class="break-all">
              {#if key}
                <!-- The name is the toggle, so the row's whole name is what
                     to tap; the arrow is decoration, out of its name. -->
                <button
                  class="text-left hover:text-foreground"
                  aria-expanded={!!breakdown}
                  aria-controls="movegens-{i}"
                  on:click={() => toggle(key)}
                  ><span aria-hidden="true" class="mr-1 inline-block w-3 text-muted-foreground"
                    >{breakdown ? '▾' : '▸'}</span
                  >{workerLabel(worker)}</button
                >
              {:else}
                {workerLabel(worker)}
              {/if}
            </td>
            {#each columns as column}
              <td
                class="whitespace-nowrap text-right tabular-nums {secondary(column.sort)}"
                data-column={column.sort}
                title={column.sort === 'compute'
                  ? `${(worker.compute_seconds / 3600).toLocaleString(undefined, { maximumFractionDigits: 1 })} hours`
                  : undefined}
              >
                {value(worker, column.sort)}
              </td>
            {/each}
            <td class="hidden sm:table-cell">{datetime(worker.last_seen_at)}</td>
          </tr>
          {#if breakdown}
            <tr id="movegens-{i}" data-testid="contributor-movegens">
              <td></td>
              <td colspan="5">
                {#if breakdown.error}
                  <p class="text-sm text-destructive">
                    Could not load the movegens by job type: {breakdown.error}
                  </p>
                {/if}
                {#if breakdown.value}
                  <dl class="grid max-w-3xl gap-x-6 gap-y-1 text-sm sm:grid-cols-2">
                    {#each movegensLines(breakdown.value) as line}
                      <div class="flex min-w-0 flex-wrap justify-between gap-x-3">
                        <dt class="text-muted-foreground">{line.label}</dt>
                        <dd class="break-all tabular-nums">{exactCount(line.movegens)}</dd>
                      </div>
                    {/each}
                  </dl>
                {:else if !breakdown.error}
                  <p class="text-sm text-muted-foreground">Loading…</p>
                {/if}
              </td>
            </tr>
          {/if}
        {:else}
          <tr><td colspan="6" class="text-muted-foreground">No contributions yet.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
  <Pagination page={result.page} perPage={result.per_page} total={result.total} onChange={show} />
{/if}
