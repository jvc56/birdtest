<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    api,
    type Contributor,
    type ContributorSort,
    type ContributionsByType,
    type Page
  } from '$lib/api';
  import { computeTime, datetime, exactCount, workerLabel } from '$lib/format';
  import { contributionLines, contributionTotals, contributorKey } from '$lib/movegens';
  import { createPoller } from '$lib/poller';
  import Pagination from '$lib/components/Pagination.svelte';
  import Count from '$lib/components/Count.svelte';

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
  // The site's work by job type, read with each page of the list. Null
  // until the first answer; a refresh that fails keeps the last.
  let siteTotals: ContributionsByType | null = null;
  $: siteLines = siteTotals ? contributionLines(siteTotals) : [];
  $: siteTotal = contributionTotals(siteLines);

  /** One contributor's breakdown by job type, open under their row. */
  interface Breakdown {
    value: ContributionsByType | null;
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
      const value = await api.contributorContributions(key);
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
    const totals = api.siteContributions().then(
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

<h1 class="mb-2 text-2xl font-semibold">Contributions</h1>
<p class="mb-6 text-sm text-muted-foreground">
  What every worker has done for the site, authenticated or anonymous, in three figures: movegens,
  the move generations MAGPIE performed, summed over every task finished; compute time, how long
  the claims were held, from claim to result; and the tasks themselves. First the site's totals and
  the same by job type, then each worker, ranked by movegens. Choose a column to rank by it
  instead, or a contributor to see their own by job type. The page refreshes itself every 30
  seconds.
</p>

{#if siteTotals}
  <!-- Every contributor's work together, then by the type of job it was for:
       the jobs' own totals, which a purged or deleted job leaves as it leaves
       the list. One figure to a line on a phone, and a count breaks between
       its digit groups, so one to its last digit never runs off the side. -->
  <section class="card mb-6 space-y-4" aria-labelledby="site-totals" data-testid="site-movegens">
    <h2 id="site-totals" class="text-sm font-medium">Site totals</h2>
    <dl class="grid gap-4 sm:grid-cols-3">
      <div class="min-w-0">
        <dt class="text-xs uppercase text-muted-foreground">Movegens</dt>
        <dd class="mt-1 text-lg tabular-nums"><Count value={siteTotal.movegens} /></dd>
      </div>
      <div class="min-w-0">
        <dt class="text-xs uppercase text-muted-foreground">Compute time</dt>
        <dd class="mt-1 text-lg tabular-nums">{computeTime(siteTotal.compute_seconds)}</dd>
      </div>
      <div class="min-w-0">
        <dt class="text-xs uppercase text-muted-foreground">Tasks</dt>
        <dd class="mt-1 text-lg tabular-nums"><Count value={siteTotal.tasks} /></dd>
      </div>
    </dl>
    <table class="table w-full table-fixed text-sm" data-testid="site-by-type">
      <thead>
        <tr>
          <th>Job type</th>
          <th class="text-right">Movegens</th>
          <th class="text-right">Compute time</th>
          <th class="text-right">Tasks</th>
        </tr>
      </thead>
      <tbody>
        {#each siteLines as line}
          <tr data-type={line.type}>
            <th scope="row" class="border-border/50 font-normal">{line.label}</th>
            <td class="text-right tabular-nums" data-figure="movegens"><Count value={line.movegens} /></td>
            <td class="text-right tabular-nums" data-figure="compute">{computeTime(line.compute_seconds)}</td>
            <td class="text-right tabular-nums" data-figure="tasks"><Count value={line.tasks} /></td>
          </tr>
        {/each}
      </tbody>
    </table>
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
  <div class="card overflow-x-auto p-0" data-testid="contributor-ranking">
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
                    Could not load the work by job type: {breakdown.error}
                  </p>
                {/if}
                {#if breakdown.value}
                  <!-- A row per job type under the same headers as the site's
                       totals. Fixed columns whose counts break between
                       digit groups, so it never widens the ranking's box on a
                       phone. -->
                  <table class="w-full max-w-3xl table-fixed text-sm">
                    <thead>
                      <tr class="text-xs text-muted-foreground">
                        <th class="py-1 text-left font-normal">Job type</th>
                        <th class="py-1 text-right font-normal">Movegens</th>
                        <th class="py-1 text-right font-normal">Compute time</th>
                        <th class="py-1 text-right font-normal">Tasks</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each contributionLines(breakdown.value) as line}
                        <tr data-type={line.type}>
                          <th scope="row" class="border-border/50 py-0.5 text-left font-normal">{line.label}</th>
                          <td class="py-0.5 text-right tabular-nums" data-figure="movegens"
                            ><Count value={line.movegens} /></td
                          >
                          <td class="py-0.5 text-right tabular-nums" data-figure="compute"
                            >{computeTime(line.compute_seconds)}</td
                          >
                          <td class="py-0.5 text-right tabular-nums" data-figure="tasks"
                            ><Count value={line.tasks} /></td
                          >
                        </tr>
                      {/each}
                    </tbody>
                  </table>
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
