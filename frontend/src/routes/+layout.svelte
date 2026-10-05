<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { session, refreshSession, signOut } from '$lib/auth';

  onMount(refreshSession);

  const links = [
    { href: '/jobs', label: 'Jobs' },
    { href: '/ratings', label: 'Ratings' },
    { href: '/player-configs', label: 'Players' },
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
    <!-- Wraps on a phone: in one row the links ran past the screen, and a
         phone's browser widened the page to fit them rather than clip. -->
    <nav class="mx-auto flex max-w-6xl flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3 sm:px-6">
      <a href="/" class="text-lg font-semibold text-foreground no-underline hover:no-underline">
        birdtest
      </a>
      <div class="flex flex-1 basis-full flex-wrap gap-x-4 gap-y-1 text-sm sm:basis-auto">
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
          <a
            href="/account"
            class="min-w-0 max-w-[40vw] truncate no-underline text-muted-foreground hover:text-foreground"
          >
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

  <main class="mx-auto w-full max-w-6xl flex-1 px-4 py-8 sm:px-6">
    <slot />
  </main>

  <footer class="border-t border-border px-6 py-4 text-center text-xs text-muted-foreground">
    birdtest — crowdsourced crossword game analysis
  </footer>
</div>
