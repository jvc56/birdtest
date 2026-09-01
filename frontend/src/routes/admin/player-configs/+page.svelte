<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type PlayerConfig } from '$lib/api';

  let configs: PlayerConfig[] = [];
  let error = '';

  async function load() {
    configs = await api.playerConfigs();
  }
  onMount(load);

  async function remove(config: PlayerConfig) {
    error = '';
    if (!confirm(`Delete "${config.name}"?`)) return;
    try {
      await api.deletePlayerConfig(config.id);
      await load();
    } catch (e) {
      error = (e as Error).message;
    }
  }
</script>

<div class="mb-4 flex items-center justify-between">
  <h1 class="text-2xl font-semibold">Player configs</h1>
  <a href="/admin/player-configs/new" class="btn-primary no-underline hover:no-underline">New</a>
</div>
<p class="mb-4 text-sm text-muted-foreground">
  Configs are immutable once created — there is no edit endpoint. Deletion is only allowed while no
  job references them.
</p>
{#if error}<p class="mb-4 text-destructive">{error}</p>{/if}

<div class="card overflow-x-auto p-0">
  <table class="table">
    <thead>
      <tr>
        <th>Name</th><th>Recorder</th><th>Sort</th><th>Leaves</th>
        <th class="text-right">Iterations</th><th class="text-right">Plies</th><th></th>
      </tr>
    </thead>
    <tbody>
      {#each configs as config}
        <tr>
          <td>{config.name}</td>
          <td>{config.recorder_type}</td>
          <td>{config.sort_strategy ?? '—'}</td>
          <td>{config.leaves ?? 'lexicon default'}</td>
          <td class="text-right tabular-nums">{config.max_iterations ?? '—'}</td>
          <td class="text-right tabular-nums">{config.plies ?? '—'}</td>
          <td class="text-right">
            <button class="btn-destructive" on:click={() => remove(config)}>Delete</button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="7" class="text-muted-foreground">No player configs yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>
