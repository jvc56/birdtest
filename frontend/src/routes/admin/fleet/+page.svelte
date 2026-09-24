<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type FleetVersion } from '$lib/api';

  let versions: FleetVersion[] = [];
  let loadError = '';

  onMount(async () => {
    try {
      versions = await api.fleet();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    }
  });
</script>

<h1 class="mb-2 text-2xl font-semibold">Fleet</h1>
<p class="mb-6 text-sm text-muted-foreground">
  What contributors claimed with over the last seven days. This is the evidence for raising a job's
  minimum MAGPIE version: keeping birdtest's pinned data in step with what released MAGPIE installs
  is a human decision, and this is half of what it is made from — the other half is each job's data
  gaps.
</p>

{#if loadError}
  <p class="mb-4 text-sm text-destructive">Could not load the fleet: {loadError}</p>
{/if}

<div class="card overflow-x-auto p-0">
  <table class="table">
    <thead>
      <tr><th>MAGPIE version</th><th class="text-right">Workers</th><th class="text-right">Claims</th></tr>
    </thead>
    <tbody>
      {#each versions as version}
        <tr>
          <td>{version.magpie_version ?? 'unreported'}</td>
          <td class="text-right tabular-nums">{version.workers}</td>
          <td class="text-right tabular-nums">{version.claims}</td>
        </tr>
      {:else}
        <tr><td colspan="3" class="text-muted-foreground">No claims in the last week.</td></tr>
      {/each}
    </tbody>
  </table>
</div>
