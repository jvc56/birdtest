<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type RatingPoolListItem } from '$lib/api';

  let pools: RatingPoolListItem[] = [];
  let loaded = false;

  onMount(async () => {
    pools = await api.ratingPools();
    loaded = true;
  });
</script>

<section class="space-y-6">
  <div class="space-y-2">
    <h1 class="text-2xl font-semibold">Ratings</h1>
    <p class="max-w-3xl text-sm text-muted-foreground">
      A rating pool is a set of player configs whose ratings are comparable, plus the conditions
      that make them so. Every rating in a pool is solved for jointly from every game pair its
      members have played, anchored on one config at a fixed rating — so ratings are meaningful
      within a pool and not between pools.
    </p>
  </div>

  {#if !loaded}
    <p class="text-sm text-muted-foreground">Loading…</p>
  {:else if !pools.length}
    <div class="card space-y-2">
      <h2 class="text-lg font-medium">No rating pools yet</h2>
      <p class="text-sm text-muted-foreground">
        An admin creates a pool by naming its variant, letter distribution and board layout, then
        picking the anchor config that fixes the scale.
      </p>
    </div>
  {:else}
    <table class="table">
      <thead>
        <tr>
          <th>Pool</th>
          <th>Conditions</th>
          <th class="text-right">Configs</th>
          <th class="text-right">Last computed</th>
        </tr>
      </thead>
      <tbody>
        {#each pools as pool}
          <tr>
            <td><a href="/ratings/{pool.id}">{pool.name}</a></td>
            <td class="text-muted-foreground">
              {pool.variant} · {pool.letter_distribution} · {pool.layout}
            </td>
            <td class="text-right tabular-nums">{pool.members}</td>
            <td class="text-right tabular-nums text-muted-foreground">
              {pool.last_computed_at
                ? new Date(pool.last_computed_at).toLocaleString()
                : 'never'}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>
