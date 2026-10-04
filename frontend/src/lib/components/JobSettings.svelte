<script lang="ts">
  /**
   * What a job runs with, as two cards: the job's own settings and its
   * type's, in one ordered list, then its players' -- one column for a single
   * player (an opening-rack or leave job's), two side by side for a games or
   * pairs job, those they differ in bold, those the job never reads muted.
   * Each card shows its key rows and has its own "All settings" toggle, so
   * either can be opened in full on the page. The same as JSON to download.
   * Public, like the rest of the page.
   */
  import { jobSettings, unusedPlayerSettings, type JobConfig } from '$lib/jobSettings';
  import PlayerSettingsTable from './PlayerSettingsTable.svelte';

  export let config: JobConfig;

  let allJob = false;
  let allPlayers = false;

  $: rows = jobSettings(config).filter((row) => allJob || row.key);
</script>

<div class="card space-y-4" data-testid="job-settings">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <h2 class="text-lg font-medium">Job settings</h2>
    <button type="button" class="btn-secondary" aria-expanded={allJob} on:click={() => (allJob = !allJob)}>
      {allJob ? 'Key settings only' : 'All settings'}
    </button>
  </div>

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

  <p class="text-xs">
    <a href="/api/jobs/{config.job.id}/config" download="job-{config.job.id}-settings.json"
      >Download every setting as JSON</a
    >
  </p>
</div>

{#if config.players.length}
  <div class="card space-y-4" data-testid="player-settings">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <h2 class="text-lg font-medium">Player settings</h2>
      <button
        type="button"
        class="btn-secondary"
        aria-expanded={allPlayers}
        on:click={() => (allPlayers = !allPlayers)}
      >
        {allPlayers ? 'Key settings only' : 'All settings'}
      </button>
    </div>
    <PlayerSettingsTable
      players={config.players}
      all={allPlayers}
      unused={unusedPlayerSettings(config)}
    />
  </div>
{/if}
