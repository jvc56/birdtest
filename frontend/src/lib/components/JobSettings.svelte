<script lang="ts">
  /**
   * What a job runs with, as two cards: the job's own settings and its
   * type's, every one of them, in one ordered list, then its players' -- one
   * column for a single player (an opening-rack or leave job's), two side by
   * side for a games or pairs job, those the job never reads muted. Two
   * players' card shows only the settings they differ in, with an "All
   * settings" toggle for every one; one player's shows its key rows, with the
   * same toggle. Public, like the rest of the page.
   */
  import { jobSettings, unusedPlayerSettings, type JobConfig, type SettingsMode } from '$lib/jobSettings';
  import PlayerSettingsTable from './PlayerSettingsTable.svelte';

  export let config: JobConfig;

  let allPlayers = false;

  // One player differs from nothing: it reads its key rows instead.
  $: paired = config.players.length > 1;
  let mode: SettingsMode;
  $: mode = allPlayers ? 'all' : paired ? 'differences' : 'key';

  $: rows = jobSettings(config);
</script>

<div class="card space-y-4" data-testid="job-settings">
  <h2 class="text-lg font-medium">Job settings</h2>

  <div class="overflow-x-auto">
    <table class="table text-sm">
      <tbody>
        {#each rows as row}
          <tr>
            <td class="w-1/3 text-muted-foreground">{row.label}</td>
            <td class="break-words tabular-nums">{row.value}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>

</div>

{#if config.players.length}
  <div class="card space-y-4" data-testid="player-settings">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <h2 class="text-lg font-medium">Player settings</h2>
      <button
        type="button"
        class="btn-secondary"
        aria-pressed={allPlayers}
        on:click={() => (allPlayers = !allPlayers)}
      >
        {allPlayers ? (paired ? 'Different settings only' : 'Key settings only') : 'All settings'}
      </button>
    </div>
    <PlayerSettingsTable players={config.players} {mode} unused={unusedPlayerSettings(config)} />
  </div>
{/if}
