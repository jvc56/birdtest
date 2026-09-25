<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { api } from '$lib/api';
  import { refreshSession } from '$lib/auth';

  let username = '';
  let password = '';
  let error = '';
  let busy = false;

  async function submit() {
    busy = true;
    error = '';
    try {
      await api.login({ username, password });
      await refreshSession();
      // Come back to whatever the user was trying to reach before the guard
      // redirected them here -- a path on this site only: `goto` refuses
      // anything else, which left a signed-in user on this page, unmoved.
      // Resolved rather than pattern-matched: `/\evil.com` and a path with a
      // tab in it both start with a single '/' and resolve off-site.
      const next = new URL($page.url.searchParams.get('next') ?? '/account', $page.url.origin);
      await goto(
        next.origin === $page.url.origin ? next.pathname + next.search + next.hash : '/account'
      );
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<div class="mx-auto max-w-md">
  <h1 class="mb-6 text-2xl font-semibold">Sign in</h1>
  <form class="card space-y-4" on:submit|preventDefault={submit}>
    <div>
      <label class="label" for="username">Username</label>
      <input id="username" class="input" bind:value={username} autocomplete="username" required />
    </div>
    <div>
      <label class="label" for="password">Password</label>
      <input
        id="password"
        type="password"
        class="input"
        bind:value={password}
        autocomplete="current-password"
        required
      />
    </div>
    {#if error}<p class="field-error">{error}</p>{/if}
    <button class="btn-primary w-full" disabled={busy}>{busy ? 'Signing in…' : 'Sign in'}</button>
    <div class="flex justify-between text-sm text-muted-foreground">
      <a href="/reset-password">Forgot password?</a>
      <a href="/register">Create an account</a>
    </div>
  </form>
</div>
