<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api';
  import { playerSummary, type PublicPlayerConfig } from '$lib/jobSettings';

  let configs: PublicPlayerConfig[] = [];
  let loaded = false;
  let error = '';

  onMount(async () => {
    try {
      configs = await api.publicPlayerConfigs();
      loaded = true;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  });
</script>

<section class="space-y-6">
  <div class="space-y-2">
    <h1 class="text-2xl font-semibold">Player configs</h1>
    <p class="max-w-3xl text-sm text-muted-foreground">
      A player config is one bot: the lexicon and leaves it plays with, how it searches for a
      move, and what it reports. Jobs pit configs against each other, and ratings are about them.
      A config never changes once made, so a result always says exactly what played.
    </p>
  </div>

  {#if error}
    <p class="text-sm text-destructive">Could not load the player configs: {error}</p>
  {:else if !loaded}
    <p class="text-sm text-muted-foreground">Loading…</p>
  {:else if !configs.length}
    <p class="text-sm text-muted-foreground">No player configs yet.</p>
  {:else}
    <div class="card overflow-x-auto p-0">
      <table class="table">
        <thead>
          <tr><th>Name</th><th>Search</th><th>Lexicon</th><th>Leaves</th><th class="text-right">Created</th></tr>
        </thead>
        <tbody>
          {#each configs as config}
            <tr>
              <td class="break-all"><a href="/player-configs/{config.id}">{config.name}</a></td>
              <td>{playerSummary(config)}</td>
              <td>{config.lexicon}</td>
              <td>{config.leaves}</td>
              <td class="text-right tabular-nums text-muted-foreground">
                {new Date(config.created_at).toLocaleDateString()}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>
