<script lang="ts">
  import { page } from '$app/stores';
  import { api } from '$lib/api';
  import PlayerSettingsTable from '$lib/components/PlayerSettingsTable.svelte';
  import { playerSummary, type PublicPlayerConfig } from '$lib/jobSettings';

  let config: PublicPlayerConfig | null = null;
  let clonedFrom: PublicPlayerConfig | null = null;
  let error = '';
  /** Every setting rather than the key rows. */
  let all = false;

  $: configId = $page.params.id as string;

  async function load(id: string) {
    error = '';
    config = null;
    clonedFrom = null;
    try {
      config = await api.publicPlayerConfig(id);
      if (config.cloned_from_id) {
        clonedFrom = await api.publicPlayerConfig(config.cloned_from_id).catch(() => null);
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  // On the id rather than once: a link to the config it was cloned from
  // stays on this route.
  $: if (configId) load(configId);
</script>

{#if error}
  <p class="text-destructive">{error}</p>
{:else if !config}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <section class="space-y-6">
    <div class="space-y-1">
      <h1 class="break-all text-2xl font-semibold">{config.name}</h1>
      <p class="text-sm text-muted-foreground">
        {playerSummary(config)} · created {new Date(config.created_at).toLocaleString()}
        {#if config.cloned_from_id}
          · cloned from
          {#if clonedFrom}<a href="/player-configs/{clonedFrom.id}">{clonedFrom.name}</a>{:else}a deleted config{/if}
          onto newer data; ratings restart on new data
        {/if}
      </p>
    </div>

    <div class="card space-y-4">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 class="text-lg font-medium">Settings</h2>
        <button type="button" class="btn-secondary" aria-expanded={all} on:click={() => (all = !all)}>
          {all ? 'Key settings only' : 'All settings'}
        </button>
      </div>
      <PlayerSettingsTable players={[config]} {all} />
      <p class="text-xs">
        <a href="/api/player-configs/{config.id}" download="player-config-{config.id}.json"
          >Download every setting as JSON</a
        >
      </p>
    </div>
  </section>
{/if}
