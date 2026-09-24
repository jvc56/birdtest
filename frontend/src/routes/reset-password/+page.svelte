<script lang="ts">
  import { api } from '$lib/api';

  let email = '';
  let sent = false;
  let busy = false;
  let error = '';

  async function submit() {
    busy = true;
    error = '';
    try {
      await api.requestPasswordReset(email);
      // The server answers 200 either way so this page cannot be used to find
      // out which addresses have accounts.
      sent = true;
    } catch (e) {
      // A refusal (a rate limit, the server down) says nothing about the
      // address -- the limiter answers the same for every one -- and claiming
      // a link is on its way when none was sent would be a lie.
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="mx-auto max-w-md">
  <h1 class="mb-6 text-2xl font-semibold">Reset your password</h1>
  {#if sent}
    <div class="card">
      <p class="text-muted-foreground">
        If that address has a confirmed account, a reset link is on its way. The link expires in 30
        minutes.
      </p>
    </div>
  {:else}
    {#if error}<p class="field-error mb-4">{error}</p>{/if}
    <form class="card space-y-4" on:submit|preventDefault={submit}>
      <div>
        <label class="label" for="email">Email</label>
        <input id="email" type="email" class="input" bind:value={email} required />
      </div>
      <button class="btn-primary w-full" disabled={busy}>
        {busy ? 'Sending…' : 'Send reset link'}
      </button>
    </form>
  {/if}
</div>
