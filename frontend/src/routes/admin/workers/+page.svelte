<script lang="ts">
  import { onMount } from 'svelte';
  import { api, errorText, type Page, type WorkerBan } from '$lib/api';
  import { workerLabel } from '$lib/format';
  import Pagination from '$lib/components/Pagination.svelte';

  let workers: Page<Record<string, any>> | null = null;
  let bans: WorkerBan[] = [];
  let target = '';
  // Stated, not guessed: a user id and an anonymous UUID are both UUIDs, and
  // guessing from the first page of contributors sent every other anonymous
  // UUID as a user id, which the server refused.
  let kind: 'anon' | 'user' = 'anon';
  let reason = '';
  let error = '';
  let notice = '';

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function loadBans() {
    bans = await api.workerBans();
  }

  // Every page, not the first: a nuisance with few completed tasks ranks far
  // down the list. (One that has completed none is not listed at all; its
  // UUID, from the contributor or the server's logs, can be typed above.)
  async function loadWorkers(next: number) {
    error = '';
    try {
      workers = await api.adminWorkers(next);
    } catch (e) {
      error = `Could not load the workers: ${message(e)}`;
    }
  }

  onMount(async () => {
    try {
      [workers] = await Promise.all([api.adminWorkers(0), loadBans()]);
    } catch (e) {
      error = `Could not load the workers: ${message(e)}`;
    }
  });

  let banning = false;

  async function ban() {
    if (banning) return;
    banning = true;
    error = '';
    notice = '';
    // Trimmed: a pasted UUID's leading space was refused as a malformed one.
    const id = target.trim();
    try {
      await api.banWorker({
        ...(kind === 'anon' ? { anon_uuid: id } : { user_id: id }),
        ...(reason ? { reason } : {})
      });
      notice = `Banned ${id}.`;
      target = '';
      reason = '';
      await loadBans();
    } catch (e) {
      // With the field the server named: "reason at most 1000 characters".
      error = errorText(e);
    } finally {
      banning = false;
    }
  }

  async function unban(entry: WorkerBan) {
    const who = entry.username ?? entry.user_id ?? entry.anon_uuid;
    // The ban row goes; its reason stays only in the audit log.
    if (!window.confirm(`Lift the ban on ${who}? Its reason stays in the audit log.`)) return;
    error = '';
    notice = '';
    try {
      await api.unbanWorker(entry.id);
      notice = `Lifted the ban on ${who}.`;
      await loadBans();
    } catch (e) {
      error = message(e);
    }
  }
</script>

<h1 class="mb-2 text-2xl font-semibold">Worker bans</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Banned identities cannot claim or submit tasks. For anonymous workers the ban targets the UUID —
  a client that sends no identity gets a new one on every claim, and a banned account's owner can
  still contribute without a key, so a ban stops neither.
</p>

<form class="card mb-6 max-w-2xl space-y-3" on:submit|preventDefault={ban}>
  <div class="flex gap-4 text-sm">
    <label><input type="radio" bind:group={kind} value="anon" /> Anonymous worker (UUID)</label>
    <label><input type="radio" bind:group={kind} value="user" /> Account (user id)</label>
  </div>
  <div>
    <label class="label" for="target">{kind === 'anon' ? 'Anonymous UUID' : 'User ID'}</label>
    <input id="target" class="input font-mono text-xs" bind:value={target} required />
  </div>
  <div>
    <label class="label" for="reason">Reason</label>
    <input id="reason" class="input" bind:value={reason} placeholder="optional" maxlength="1000" />
  </div>
  {#if error}<p class="field-error">{error}</p>{/if}
  {#if notice}<p class="text-sm text-success [overflow-wrap:anywhere]">{notice}</p>{/if}
  <button class="btn-destructive">Ban worker</button>
</form>

<h2 class="mb-3 text-lg font-medium">Bans in force</h2>
<div class="card mb-6 overflow-x-auto p-0">
  <table class="table">
    <thead>
      <tr><th>Identity</th><th>Reason</th><th>Since</th><th></th></tr>
    </thead>
    <tbody>
      {#each bans as entry}
        <tr>
          <td class="font-mono text-xs">{entry.username ?? entry.user_id ?? entry.anon_uuid}</td>
          <td>{entry.reason ?? ''}</td>
          <td class="text-xs">{new Date(entry.created_at).toLocaleString()}</td>
          <td class="text-right">
            <button class="btn-secondary" on:click={() => unban(entry)}>Unban</button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="4" class="text-muted-foreground">No bans.</td></tr>
      {/each}
    </tbody>
  </table>
</div>

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
              on:click={() => {
                target = worker.user_id ?? worker.anon_uuid;
                kind = worker.user_id ? 'user' : 'anon';
              }}
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
{#if workers}
  <Pagination page={workers.page} perPage={workers.per_page} total={workers.total} onChange={loadWorkers} />
{/if}
