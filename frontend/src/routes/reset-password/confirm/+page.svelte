<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api, ApiError } from '$lib/api';

  let password = '';
  let error = '';
  let busy = false;

  $: token = $page.url.searchParams.get('token') ?? '';

  async function submit() {
    busy = true;
    error = '';
    try {
      await api.confirmPasswordReset(token, password);
      goto('/login');
    } catch (e) {
      error = e instanceof ApiError ? (e.fields.password ?? e.message) : (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<div class="mx-auto max-w-md">
  <h1 class="mb-6 text-2xl font-semibold">Choose a new password</h1>
  {#if !token}
    <p class="text-destructive">That link is missing its reset token.</p>
  {:else}
    <form class="card space-y-4" on:submit|preventDefault={submit}>
      <div>
        <label class="label" for="password">New password</label>
        <input
          id="password"
          type="password"
          class="input"
          bind:value={password}
          autocomplete="new-password"
          required
        />
      </div>
      {#if error}<p class="field-error">{error}</p>{/if}
      <button class="btn-primary w-full" disabled={busy}>
        {busy ? 'Saving…' : 'Set new password'}
      </button>
    </form>
  {/if}
</div>
