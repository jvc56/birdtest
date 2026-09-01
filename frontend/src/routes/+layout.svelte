<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { session, refreshSession, signOut } from '$lib/auth';

  onMount(refreshSession);

  const links = [
    { href: '/jobs', label: 'Jobs' },
    { href: '/workers', label: 'Contributors' },
    { href: '/users', label: 'Users' }
  ];

  async function handleSignOut() {
    await signOut();
    goto('/');
  }
</script>

<div class="flex min-h-screen flex-col">
  <header class="border-b border-border bg-card/50">
    <nav class="mx-auto flex max-w-6xl items-center gap-6 px-6 py-3">
      <a href="/" class="text-lg font-semibold text-foreground no-underline hover:no-underline">
        birdtest
      </a>
      <div class="flex flex-1 gap-4 text-sm">
        {#each links as link}
          <a
            href={link.href}
            class="no-underline {$page.url.pathname.startsWith(link.href)
              ? 'text-primary'
              : 'text-muted-foreground hover:text-foreground'}">{link.label}</a
          >
        {/each}
        {#if $session?.is_admin}
          <a
            href="/admin/jobs/new"
            class="no-underline {$page.url.pathname.startsWith('/admin')
              ? 'text-primary'
              : 'text-muted-foreground hover:text-foreground'}">Admin</a
          >
        {/if}
      </div>
      <div class="flex items-center gap-3 text-sm">
        {#if $session}
          <a href="/account" class="no-underline text-muted-foreground hover:text-foreground">
            {$session.username}
          </a>
          <button class="btn-secondary" on:click={handleSignOut}>Sign out</button>
        {:else if $session === null}
          <a href="/login" class="no-underline text-muted-foreground hover:text-foreground">
            Sign in
          </a>
          <a href="/register" class="btn-primary no-underline hover:no-underline">Register</a>
        {/if}
      </div>
    </nav>
  </header>

  <main class="mx-auto w-full max-w-6xl flex-1 px-6 py-8">
    <slot />
  </main>

  <footer class="border-t border-border px-6 py-4 text-center text-xs text-muted-foreground">
    birdtest — crowdsourced word game analysis
  </footer>
</div>
