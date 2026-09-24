<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ApiKey } from '$lib/api';
  import { goto } from '$app/navigation';
  import { session } from '$lib/auth';
  import { datetime } from '$lib/format';

  let keys: ApiKey[] = [];
  let label = '';
  let freshKey: string | null = null;
  let error = '';

  async function load() {
    try {
      keys = await api.apiKeys();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      return;
    }
  }
  onMount(load);

  async function create() {
    error = '';
    try {
      const created = await api.createApiKey(label || null);
      // Shown exactly once — only the hash is stored server-side.
      freshKey = created.key;
      label = '';
      await load();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function toggle(key: ApiKey) {
    error = '';
    try {
      await api.setApiKeyActive(key.id, !key.is_active);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    await load();
  }

  let signOutError = '';
  async function signOutEverywhere() {
    if (!confirm('Sign out of every browser and device, including this one?')) return;
    signOutError = '';
    try {
      await api.signOutEverywhere();
      session.set(null);
      await goto('/login');
    } catch (e) {
      signOutError = (e as Error).message;
    }
  }

  async function revoke(key: ApiKey) {
    if (!confirm('Permanently revoke this key? Workers using it will stop being authenticated.'))
      return;
    error = '';
    try {
      await api.revokeApiKey(key.id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    await load();
  }
</script>

<h1 class="mb-6 text-2xl font-semibold">Account</h1>

<div class="space-y-6">
  <div class="card">
    <dl class="grid grid-cols-2 gap-4 text-sm sm:grid-cols-4">
      <div><dt class="text-muted-foreground">Username</dt><dd>{$session?.username}</dd></div>
      <div><dt class="text-muted-foreground">Email</dt><dd>{$session?.email}</dd></div>
      <div><dt class="text-muted-foreground">Role</dt><dd>{$session?.is_admin ? 'admin' : 'contributor'}</dd></div>
      <div>
        <dt class="text-muted-foreground">Tasks completed</dt>
        <dd class="tabular-nums">{$session?.tasks_completed.toLocaleString()}</dd>
      </div>
    </dl>
  </div>

  <div class="card space-y-3">
    <div>
      <h2 class="text-lg font-medium">Sessions</h2>
      <p class="text-sm text-muted-foreground">
        Signs out every browser and device signed in to this account, including this one. A
        password reset does the same.
      </p>
    </div>
    <button class="btn-secondary" on:click={signOutEverywhere}>Sign out everywhere</button>
    {#if signOutError}<p class="field-error">{signOutError}</p>{/if}
  </div>

  <div class="card space-y-4">
    <div>
      <h2 class="text-lg font-medium">API keys</h2>
      <p class="text-sm text-muted-foreground">
        Pass one to the worker with <code class="rounded bg-muted px-1">--api-key</code> to credit
        your work to this account. Up to 100 keys; deactivate one to suspend it without losing it.
      </p>
    </div>

    {#if freshKey}
      <div class="rounded-md border border-warning/40 bg-warning/10 p-3">
        <p class="text-sm font-medium text-warning">Copy this key now — it is not shown again.</p>
        <code class="mt-2 block break-all font-mono text-xs">{freshKey}</code>
      </div>
    {/if}

    <form class="flex gap-2" on:submit|preventDefault={create}>
      <input class="input max-w-xs" bind:value={label} placeholder="Label (optional)" />
      <button class="btn-primary">Generate key</button>
    </form>
    {#if error}<p class="field-error">{error}</p>{/if}

    <table class="table">
      <thead>
        <tr><th>Label</th><th>Created</th><th>Last used</th><th>Status</th><th></th></tr>
      </thead>
      <tbody>
        {#each keys as key}
          <tr>
            <td>{key.label ?? '—'}</td>
            <td>{datetime(key.created_at)}</td>
            <td>{datetime(key.last_used_at)}</td>
            <td>{key.is_active ? 'active' : 'inactive'}</td>
            <td class="text-right">
              <button class="btn-secondary mr-2" on:click={() => toggle(key)}>
                {key.is_active ? 'Deactivate' : 'Activate'}
              </button>
              <button class="btn-destructive" on:click={() => revoke(key)}>Revoke</button>
            </td>
          </tr>
        {:else}
          <tr><td colspan="5" class="text-muted-foreground">No API keys yet.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>
