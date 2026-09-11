<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { session } from '$lib/auth';

  // `undefined` means the session has not resolved yet; only an explicit `null`
  // is a signed-out user worth redirecting.
  $: if ($session === null) {
    goto(`/login?next=${encodeURIComponent($page.url.pathname)}`);
  }
</script>

{#if $session}
  <slot />
{:else}
  <p class="text-muted-foreground">Loading…</p>
{/if}
