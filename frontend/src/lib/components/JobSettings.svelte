<script lang="ts">
  /**
   * What a job runs with, as two tables: the job's own settings and its
   * type's, grouped, and its players' -- one column for a single player (an
   * opening-rack or leave job's), two side by side for a games or pairs job,
   * those they differ in bold, those the job never reads muted. Each
   * shows its key rows; one "All settings" toggle opens every row of both.
   * The same as JSON to download. Public, like the rest of the page.
   */
  import { jobGroups, keyGroups, unusedPlayerSettings, type JobConfig } from '$lib/jobSettings';
  import PlayerSettingsTable from './PlayerSettingsTable.svelte';

  export let config: JobConfig;

  let all = false;

  $: groups = all ? jobGroups(config) : keyGroups(jobGroups(config));
</script>

<div class="card space-y-4">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <h2 class="text-lg font-medium">Settings</h2>
    <button type="button" class="btn-secondary" aria-expanded={all} on:click={() => (all = !all)}>
      {all ? 'Key settings only' : 'All settings'}
    </button>
  </div>

  <!-- Group titles head rows rather than being headings: a heading here named
       "Game pairs and the test" would answer the page's own "Game pairs". -->
  <div class="overflow-x-auto">
    <table class="table text-sm">
      {#each groups as group}
        <tbody>
          <tr><th colspan="2" scope="colgroup" class="bg-muted/50">{group.title}</th></tr>
          {#each group.rows as row}
            <tr>
              <td class="w-1/3 text-muted-foreground">{row.label}</td>
              <td class="break-words tabular-nums">{row.value}</td>
            </tr>
          {/each}
        </tbody>
      {/each}
    </table>
  </div>

  {#if config.players.length}
    <PlayerSettingsTable
      players={config.players}
      {all}
      unused={unusedPlayerSettings(config.job.job_type)}
    />
  {/if}

  <p class="text-xs">
    <a href="/api/jobs/{config.job.id}/config" download="job-{config.job.id}-settings.json"
      >Download every setting as JSON</a
    >
  </p>
</div>
