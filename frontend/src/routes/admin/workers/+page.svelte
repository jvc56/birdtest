<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Page } from '$lib/api';
  import { workerLabel } from '$lib/format';

  let workers: Page<Record<string, any>> | null = null;
  let target = '';
  let reason = '';
  let error = '';
  let notice = '';

  onMount(async () => {
    workers = await api.workers(0);
  });

  /** A user id and an anonymous UUID are both UUIDs, so which field to send is
   *  decided by whether the value matches a known anonymous worker. */
  async function ban() {
    error = '';
    notice = '';
    const isAnon = workers?.items.some((w) => w.anon_uuid === target);
    try {
      await api.banWorker({
        ...(isAnon ? { anon_uuid: target } : { user_id: target }),
        ...(reason ? { reason } : {})
      });
      notice = `Banned ${target}.`;
      target = '';
      reason = '';
    } catch (e) {
      error = (e as Error).message;
    }
  }
</script>

<h1 class="mb-2 text-2xl font-semibold">Worker bans</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Banned identities cannot claim or submit tasks. For anonymous workers the ban targets the UUID.
</p>

<form class="card mb-6 max-w-2xl space-y-3" on:submit|preventDefault={ban}>
  <div>
    <label class="label" for="target">User ID or anonymous UUID</label>
    <input id="target" class="input font-mono text-xs" bind:value={target} required />
  </div>
  <div>
    <label class="label" for="reason">Reason</label>
    <input id="reason" class="input" bind:value={reason} placeholder="optional" />
  </div>
  {#if error}<p class="field-error">{error}</p>{/if}
  {#if notice}<p class="text-sm text-success">{notice}</p>{/if}
  <button class="btn-destructive">Ban worker</button>
</form>

<h2 class="mb-3 text-lg font-medium">Known workers</h2>
<div class="card overflow-x-auto p-0">
  <table class="table">
    <thead>
      <tr><th>Contributor</th><th>Identifier</th><th class="text-right">Tasks</th><th></th></tr>
    </thead>
    <tbody>
      {#each workers?.items ?? [] as worker}
        <tr>
          <td>{workerLabel(worker)}</td>
          <td class="font-mono text-xs">{worker.user_id ?? worker.anon_uuid}</td>
          <td class="text-right tabular-nums">{Number(worker.tasks_completed).toLocaleString()}</td>
          <td class="text-right">
            <button
              class="btn-secondary"
              on:click={() => (target = worker.user_id ?? worker.anon_uuid)}
            >
              Select
            </button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="4" class="text-muted-foreground">No workers yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>
