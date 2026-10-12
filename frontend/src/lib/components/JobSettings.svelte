<script lang="ts">
  /**
   * What a job runs with, as two cards: the job's own settings and its
   * type's, every one of them, in one ordered list, then its players' -- one
   * column for a single player (an opening-rack or leave job's), two side by
   * side for a games or pairs job, those the job never reads muted. Two
   * players' card shows every setting, those they differ in marked and
   * counted beside the heading, with a "Different settings only" toggle; one
   * player's shows its key rows, with an "All settings" toggle. Public, like
   * the rest of the page.
   */
  import {
    differingSettings,
    jobSettings,
    playerRows,
    unusedPlayerSettings,
    type JobConfig,
    type SettingsMode
  } from '$lib/jobSettings';
  import PlayerSettingsTable from './PlayerSettingsTable.svelte';

  export let config: JobConfig;

  /** Two players: only the settings they differ in. One: every setting. */
  let toggled = false;

  // One player differs from nothing: it reads its key rows instead.
  $: paired = config.players.length > 1;
  let mode: SettingsMode;
  $: mode = paired ? (toggled ? 'differences' : 'all') : toggled ? 'all' : 'key';
  $: unused = unusedPlayerSettings(config);
  $: differing = paired ? differingSettings(config.players, unused).length : 0;
  $: listed = paired ? playerRows(config.players, unused).length : 0;

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
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="text-lg font-medium">Player settings</h2>
        {#if paired}
          <span
            class="rounded-full border border-border bg-muted px-2.5 py-0.5 text-xs tabular-nums text-muted-foreground"
            data-testid="settings-differ-count"
          >
            {differing ? `${differing} of ${listed} differ` : 'identical'}
          </span>
        {/if}
      </div>
      <button type="button" class="btn-secondary" aria-pressed={toggled} on:click={() => (toggled = !toggled)}>
        {#if paired}
          {toggled ? 'All settings' : 'Different settings only'}
        {:else}
          {toggled ? 'Key settings only' : 'All settings'}
        {/if}
      </button>
    </div>
    <PlayerSettingsTable players={config.players} {mode} {unused} />
  </div>
{/if}
