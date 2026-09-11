<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Page } from '$lib/api';
  import { datetime, workerLabel } from '$lib/format';
  import Pagination from '$lib/components/Pagination.svelte';

  let result: Page<Record<string, any>> | null = null;

  async function load(page: number) {
    result = await api.workers(page);
  }
  onMount(() => load(0));
</script>

<h1 class="mb-2 text-2xl font-semibold">Contributors</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Every worker that has completed a task, authenticated or anonymous.
</p>

{#if !result}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="card overflow-x-auto p-0">
    <table class="table">
      <thead>
        <tr><th>#</th><th>Contributor</th><th>Last result</th><th class="text-right">Tasks completed</th></tr>
      </thead>
      <tbody>
        {#each result.items as worker, i}
          <tr>
            <td class="tabular-nums text-muted-foreground">{result.page * result.per_page + i + 1}</td>
            <td>{workerLabel(worker)}</td>
            <td>{datetime(worker.last_seen_at)}</td>
            <td class="text-right tabular-nums">{Number(worker.tasks_completed).toLocaleString()}</td>
          </tr>
        {:else}
          <tr><td colspan="4" class="text-muted-foreground">No contributions yet.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
  <Pagination page={result.page} perPage={result.per_page} total={result.total} onChange={load} />
{/if}
