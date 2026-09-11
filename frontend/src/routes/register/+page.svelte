<script lang="ts">
  import { goto } from '$app/navigation';
  import { api, ApiError } from '$lib/api';

  let username = '';
  let email = '';
  let password = '';
  let fields: Record<string, string> = {};
  let error = '';
  let busy = false;

  /**
   * Client-side strength feedback only. The server scores the password again on
   * submit and is the one that decides — this is here so the user finds out
   * before they press the button.
   */
  $: strength = (() => {
    let score = 0;
    if (password.length >= 12) score++;
    if (password.length >= 16) score++;
    if (/[^A-Za-z0-9]/.test(password)) score++;
    if (/\d/.test(password) && /[A-Za-z]/.test(password)) score++;
    return Math.min(score, 4);
  })();

  const labels = ['very weak', 'weak', 'fair', 'good', 'strong'];

  async function submit() {
    busy = true;
    error = '';
    fields = {};
    try {
      await api.register({ username, email, password });
      goto('/register/check-email');
    } catch (e) {
      if (e instanceof ApiError) {
        fields = e.fields;
        error = Object.keys(e.fields).length ? '' : e.message;
      } else {
        error = (e as Error).message;
      }
    } finally {
      busy = false;
    }
  }
</script>

<div class="mx-auto max-w-md">
  <h1 class="mb-6 text-2xl font-semibold">Create an account</h1>
  <p class="mb-6 text-sm text-muted-foreground">
    An account exists to generate API tokens so your work is credited to you. Contributing does not
    require one — an anonymous worker can complete every task.
  </p>

  <form class="card space-y-4" on:submit|preventDefault={submit}>
    <div>
      <label class="label" for="username">Username</label>
      <input id="username" class="input" bind:value={username} autocomplete="username" required />
      {#if fields.username}<p class="field-error">{fields.username}</p>{/if}
    </div>
    <div>
      <label class="label" for="email">Email</label>
      <input id="email" type="email" class="input" bind:value={email} autocomplete="email" required />
      {#if fields.email}<p class="field-error">{fields.email}</p>{/if}
    </div>
    <div>
      <label class="label" for="password">Password</label>
      <input
        id="password"
        type="password"
        class="input"
        bind:value={password}
        autocomplete="new-password"
        required
      />
      {#if password}
        <p class="mt-1 text-xs text-muted-foreground">Strength: {labels[strength]}</p>
      {/if}
      {#if fields.password}<p class="field-error">{fields.password}</p>{/if}
    </div>
    {#if error}<p class="field-error">{error}</p>{/if}
    <button class="btn-primary w-full" disabled={busy}>{busy ? 'Creating…' : 'Register'}</button>
    <p class="text-center text-sm text-muted-foreground">
      Already registered? <a href="/login">Sign in</a>
    </p>
  </form>
</div>
