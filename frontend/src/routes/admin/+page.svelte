<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';

  // /admin is just an entry point; the job list is the useful landing spot.
  onMount(() => goto('/jobs'));
</script>

<p class="text-muted-foreground">Redirecting…</p>
