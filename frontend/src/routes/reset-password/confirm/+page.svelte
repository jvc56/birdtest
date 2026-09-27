<script lang="ts">
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api, ApiError } from '$lib/api';
  import { session } from '$lib/auth';

  let password = '';
  let error = '';
  let outOfTries = false;
  let waitMinutes: number | null = null;
  let busy = false;

  $: token = $page.url.searchParams.get('token') ?? '';

  async function submit() {
    busy = true;
    error = '';
    outOfTries = false;
    try {
      await api.confirmPasswordReset(token, password);
      // A reset signs out every session, this tab's included.
      session.set(null);
      goto('/login');
    } catch (e) {
      // A link buys five tries an hour, weak passwords included: past them a
      // new link starts afresh, which "too many requests" did not say.
      outOfTries = e instanceof ApiError && e.status === 429;
      waitMinutes =
        e instanceof ApiError && e.retryAfter !== null ? Math.max(1, Math.ceil(e.retryAfter / 60)) : null;
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
      {#if outOfTries}
        <p class="field-error" role="alert">
          Too many tries, with this link or from this address. Try again in
          {waitMinutes === null ? 'a few minutes' : `${waitMinutes} minute${waitMinutes === 1 ? '' : 's'}`}, or
          <a href="/reset-password">request a new link</a>.
        </p>
      {:else if error}<p class="field-error" role="alert">{error}</p>{/if}
      <button class="btn-primary w-full" disabled={busy}>
        {busy ? 'Saving…' : 'Set new password'}
      </button>
    </form>
  {/if}
</div>
