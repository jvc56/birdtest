<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, type ApiKey } from '$lib/api';
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
      // Resuming is limited per account; say how long to wait.
      const minutes =
        e instanceof ApiError && e.status === 429 && e.retryAfter !== null
          ? Math.max(1, Math.ceil(e.retryAfter / 60))
          : null;
      const wait = minutes === null ? '' : ` — try again in ${minutes} minute${minutes === 1 ? '' : 's'}.`;
      error = (e instanceof Error ? e.message : String(e)) + wait;
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
      <div class="min-w-0"><dt class="text-muted-foreground">Username</dt><dd class="break-all">{$session?.username}</dd></div>
      <div class="min-w-0"><dt class="text-muted-foreground">Email</dt><dd class="break-all">{$session?.email}</dd></div>
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
        Add one to the <code class="rounded bg-muted px-1">contribute.txt</code> you run MAGPIE with, as a
        line <code class="rounded bg-muted px-1">apikey &lt;key&gt;</code>, to credit your work to this
        account. Use one key per machine: machines sharing a key share its rate limit. Up to 100
        keys; deactivate one to suspend it without losing it.
      </p>
    </div>

    {#if freshKey}
      <div class="rounded-md border border-warning/40 bg-warning/10 p-3">
        <p class="text-sm font-medium text-warning">Copy this key now — it is not shown again.</p>
        <code data-testid="fresh-key" class="mt-2 block break-all font-mono text-xs">{freshKey}</code>
        <p class="mt-2 text-sm text-muted-foreground">Its line for contribute.txt:</p>
        <code data-testid="fresh-key-line" class="mt-1 block break-all font-mono text-xs"
          >apikey {freshKey}</code
        >
      </div>
    {/if}

    <form class="flex gap-2" on:submit|preventDefault={create}>
      <input class="input max-w-xs" bind:value={label} maxlength="100" placeholder="Label (optional)" />
      <button class="btn-primary">Generate key</button>
    </form>
    {#if error}<p class="field-error">{error}</p>{/if}

    <div class="overflow-x-auto">
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
</div>
