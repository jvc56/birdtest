<script lang="ts">
  import { page } from '$app/stores';
  import { api } from '$lib/api';
  import { keySettings, playerRows, playerSummary, type PublicPlayerConfig } from '$lib/jobSettings';

  let config: PublicPlayerConfig | null = null;
  let clonedFrom: PublicPlayerConfig | null = null;
  let error = '';

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

    <div class="card space-y-3">
      <h2 class="text-lg font-medium">Settings</h2>
      <dl class="grid grid-cols-2 gap-x-6 gap-y-2 text-sm sm:grid-cols-3">
        {#each keySettings(config) as [label, value]}
          <div>
            <dt class="text-muted-foreground">{label}</dt>
            <dd class="break-all">{value}</dd>
          </div>
        {/each}
      </dl>

      <details>
        <summary class="cursor-pointer text-sm font-medium">All settings</summary>
        <div class="mt-3 overflow-x-auto">
          <table class="table text-sm">
            <tbody>
              {#each playerRows([config]) as row}
                <tr>
                  <td class="text-muted-foreground">{row.label}</td>
                  <td class="break-all tabular-nums">{row.values[0]}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <p class="mt-2 text-xs">
          <a href="/api/player-configs/{config.id}" download="player-config-{config.id}.json"
            >Download every setting as JSON</a
          >
        </p>
      </details>
    </div>
  </section>
{/if}
