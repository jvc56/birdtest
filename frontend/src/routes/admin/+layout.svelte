<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { session } from '$lib/auth';

  const tabs = [
    { href: '/admin/jobs/new', label: 'New job' },
    { href: '/admin/allocation', label: 'Allocation' },
    { href: '/admin/player-configs', label: 'Player configs' },
    { href: '/admin/rating-pools/new', label: 'New rating pool' },
    { href: '/admin/input-data', label: 'Input data' },
    { href: '/admin/fleet', label: 'Fleet' },
    { href: '/admin/users', label: 'Users' },
    { href: '/admin/workers', label: 'Bans' },
    { href: '/admin/derived-data', label: 'Derived data' },
    { href: '/admin/backups', label: 'Backups' },
    { href: '/admin/audit-log', label: 'Audit log' }
  ];

  // Only an explicitly resolved session is acted on; `undefined` is still
  // loading, or the server could not answer yet (a deploy's 503: lib/auth.ts
  // asks again). Signed out -- a lapsed session, "sign out everywhere" in another
  // tab -- goes to sign in and comes back here (an import in progress is
  // picked up again on return); signed in but not an admin goes home.
  $: if ($session === null) goto(`/login?next=${encodeURIComponent($page.url.pathname)}`);
  $: if ($session && !$session.is_admin) goto('/');
</script>

{#if $session?.is_admin}
  <div class="mb-6 flex flex-wrap gap-x-4 gap-y-2 border-b border-border pb-3 text-sm">
    {#each tabs as tab}
      <a href={tab.href} class="text-muted-foreground no-underline hover:text-foreground">
        {tab.label}
      </a>
    {/each}
  </div>
  <slot />
{:else}
  <p class="text-muted-foreground">Loading…</p>
{/if}
