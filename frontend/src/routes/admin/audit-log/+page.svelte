<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Page } from '$lib/api';
  import { datetime } from '$lib/format';
  import Pagination from '$lib/components/Pagination.svelte';

  let result: Page<Record<string, any>> | null = null;
  let action = '';
  let targetType = '';
  let page = 0;

  async function load(next: number) {
    page = next;
    result = await api.auditLog({
      page: next,
      ...(action ? { action } : {}),
      ...(targetType ? { target_type: targetType } : {})
    });
  }
  onMount(() => load(0));
</script>

<h1 class="mb-4 text-2xl font-semibold">Audit log</h1>

<form class="mb-4 flex flex-wrap items-end gap-3" on:submit|preventDefault={() => load(0)}>
  <div>
    <label class="label" for="action">Action</label>
    <input id="action" class="input w-56" bind:value={action} placeholder="job.activated" />
  </div>
  <div>
    <label class="label" for="target">Target type</label>
    <input id="target" class="input w-40" bind:value={targetType} placeholder="job" />
  </div>
  <button class="btn-primary">Filter</button>
</form>

{#if !result}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="card overflow-x-auto p-0">
    <table class="table">
      <thead>
        <tr><th>When</th><th>Action</th><th>Actor</th><th>Target</th><th>Detail</th></tr>
      </thead>
      <tbody>
        {#each result.items as row}
          <tr>
            <td class="whitespace-nowrap">{datetime(row.created_at)}</td>
            <td class="font-mono text-xs">{row.action}</td>
            <td class="font-mono text-xs">
              {(row.actor_user_id ?? row.actor_anon_uuid ?? '—').toString().slice(0, 8)}
            </td>
            <td class="font-mono text-xs">
              {row.target_type ?? '—'}
              {row.target_id ? ` ${String(row.target_id).slice(0, 8)}` : ''}
            </td>
            <td class="text-xs text-muted-foreground">
              {#if row.old_status || row.new_status}{row.old_status} → {row.new_status}{/if}
              {row.reason ?? ''}
            </td>
          </tr>
        {:else}
          <tr><td colspan="5" class="text-muted-foreground">No entries.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
  <Pagination page={result.page} perPage={result.per_page} total={result.total} onChange={load} />
{/if}
