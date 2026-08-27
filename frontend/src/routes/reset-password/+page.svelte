<script lang="ts">
  import { api } from '$lib/api';

  let email = '';
  let sent = false;
  let busy = false;

  async function submit() {
    busy = true;
    try {
      await api.requestPasswordReset(email);
    } finally {
      // The server answers 200 either way so this page cannot be used to find
      // out which addresses have accounts.
      sent = true;
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
