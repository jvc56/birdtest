<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type InputData, type PlayerConfig } from '$lib/api';

  let configs: PlayerConfig[] = [];
  let files: InputData[] = [];
  let error = '';

  // Configs pin rows; the table shows the names those rows carry.
  $: byId = new Map(files.map((f) => [f.id, f]));
  $: byConfigId = new Map(configs.map((c) => [c.id, c]));

  function fileName(id: string | null): string {
    if (!id) return '—';
    const file = byId.get(id);
    return file ? `${file.name} (${file.tarball_date})` : '—';
  }

  async function load() {
    [configs, files] = await Promise.all([api.playerConfigs(), api.inputData()]);
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
  job references them. A config pins exact file contents, so updating data means cloning the config
  onto the new rows rather than editing this one.
</p>
{#if error}<p class="mb-4 text-destructive">{error}</p>{/if}

<div class="card overflow-x-auto p-0">
  <table class="table">
    <thead>
      <tr>
        <th>Name</th><th>Recorder</th><th>Sort</th><th>Lexicon</th><th>Leaves</th>
        <th>Win%</th><th class="text-right">Iterations</th><th class="text-right">Plies</th><th></th>
      </tr>
    </thead>
    <tbody>
      {#each configs as config}
        <tr>
          <td>{config.name}</td>
          <td>{config.recorder_type}</td>
          <td>{config.sort_strategy ?? '—'}</td>
          <td>{fileName(config.kwg_id)}</td>
          <td>{fileName(config.klv_id)}</td>
          <td>{fileName(config.winpct_id)}</td>
          <td class="text-right tabular-nums">{config.max_iterations ?? '—'}</td>
          <td class="text-right tabular-nums">{config.num_plies ?? '—'}</td>
          <td class="text-right">
            <button class="btn-destructive" on:click={() => remove(config)}>Delete</button>
          </td>
        </tr>
        {#if config.cloned_from_id}
          <tr>
            <td colspan="9" class="text-xs text-muted-foreground">
              Cloned from {byConfigId.get(config.cloned_from_id)?.name ?? 'a deleted config'} —
              ratings restart on new data, because they are only comparable
              between players that ran on identical files.
            </td>
          </tr>
        {/if}
      {:else}
        <tr><td colspan="9" class="text-muted-foreground">No player configs yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>
