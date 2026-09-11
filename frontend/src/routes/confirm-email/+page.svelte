<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api } from '$lib/api';

  let state: 'working' | 'done' | 'error' = 'working';
  let message = '';

  onMount(async () => {
    const code = $page.url.searchParams.get('code');
    if (!code) {
      state = 'error';
      message = 'That link is missing its confirmation code.';
      return;
    }
    try {
      await api.confirmEmail(code);
      state = 'done';
      setTimeout(() => goto('/login'), 1500);
    } catch (e) {
      state = 'error';
      message = (e as Error).message;
    }
  });
</script>

<div class="mx-auto max-w-md text-center">
  {#if state === 'working'}
    <p class="text-muted-foreground">Confirming your email…</p>
  {:else if state === 'done'}
    <h1 class="mb-2 text-2xl font-semibold">Email confirmed</h1>
    <p class="text-muted-foreground">Taking you to sign in…</p>
  {:else}
    <h1 class="mb-2 text-2xl font-semibold">Could not confirm</h1>
    <p class="text-destructive">{message}</p>
    <a href="/register" class="btn-secondary mt-6 inline-flex no-underline hover:no-underline">
      Register again
    </a>
  {/if}
</div>
