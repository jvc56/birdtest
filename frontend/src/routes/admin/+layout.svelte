<script lang="ts">
  import { goto } from '$app/navigation';
  import { session } from '$lib/auth';

  const tabs = [
    { href: '/admin/jobs/new', label: 'New job' },
    { href: '/admin/player-configs', label: 'Player configs' },
    { href: '/admin/users', label: 'Users' },
    { href: '/admin/workers', label: 'Bans' },
    { href: '/admin/audit-log', label: 'Audit log' }
  ];

  // Only an explicitly resolved non-admin gets bounced; `undefined` is still loading.
  $: if ($session === null || ($session && !$session.is_admin)) goto('/');
</script>

{#if $session?.is_admin}
  <div class="mb-6 flex gap-4 border-b border-border pb-3 text-sm">
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
