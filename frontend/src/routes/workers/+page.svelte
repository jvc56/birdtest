<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Contributor, type ContributorSort, type Page } from '$lib/api';
  import { computeTime, datetime, workerLabel } from '$lib/format';
  import Pagination from '$lib/components/Pagination.svelte';

  let result: Page<Contributor> | null = null;
  let loadError = '';
  // Compute time by default: MAGPIE reports no CPU time, and the time a claim
  // was held is the fairest measure there is of what a machine gave -- one
  // that plays deep, slow games finishes few tasks in many hours.
  let sort: ContributorSort = 'compute';
  // Each load is numbered, and only the newest one's answer is shown: a
  // header clicked while the previous order is still loading must not be
  // overwritten by it.
  let generation = 0;

  const columns: { sort: ContributorSort; label: string }[] = [
    { sort: 'compute', label: 'Compute time' },
    { sort: 'games', label: 'Games' },
    { sort: 'racks', label: 'Racks' },
    { sort: 'tasks', label: 'Tasks' }
  ];

  async function load(page: number) {
    const mine = ++generation;
    loadError = '';
    try {
      const next = await api.workers(page, sort);
      if (mine === generation) result = next;
    } catch (e) {
      if (mine === generation) loadError = e instanceof Error ? e.message : String(e);
    }
  }

  function rankBy(next: ContributorSort) {
    if (next === sort) return;
    sort = next;
    load(0);
  }

  function value(worker: Contributor, column: ContributorSort): string {
    switch (column) {
      case 'compute':
        return computeTime(worker.compute_seconds);
      case 'games':
        return worker.games_played.toLocaleString();
      case 'racks':
        return worker.racks_analyzed.toLocaleString();
      case 'tasks':
        return worker.tasks_completed.toLocaleString();
    }
  }

  // On a phone only the column the list is ranked by is shown beside the
  // name: with a pseudonym's sixteen characters, any second one pushed it out
  // of the box.
  $: secondary = (column: ContributorSort) => (column === sort ? '' : 'hidden sm:table-cell');

  onMount(() => load(0));
</script>

<h1 class="mb-2 text-2xl font-semibold">Contributors</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Every worker that has completed a task, authenticated or anonymous. Compute time is how long its
  claims were held, from claim to result, summed over every task it finished; choose a column to
  rank by it instead.
</p>

{#if loadError}
  <p class="text-sm text-destructive">Could not load the contributors: {loadError}</p>
{:else if !result}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="card overflow-x-auto p-0">
    <table class="table">
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
          <tr>
            <td class="tabular-nums text-muted-foreground">{result.page * result.per_page + i + 1}</td>
            <td class="break-all">{workerLabel(worker)}</td>
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
        {:else}
          <tr><td colspan="7" class="text-muted-foreground">No contributions yet.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
  <Pagination page={result.page} perPage={result.per_page} total={result.total} onChange={load} />
{/if}
