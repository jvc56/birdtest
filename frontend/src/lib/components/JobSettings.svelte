<script lang="ts">
  /**
   * What a job runs with: a line per player naming how it searches, always
   * shown, and every setting -- the job's, its type's and each player's side
   * by side, those the players differ in marked -- behind "All settings",
   * with the same as JSON to download. Public, like the rest of the page.
   */
  import { jobGroups, playerRows, playerSummary, type JobConfig } from '$lib/jobSettings';

  export let config: JobConfig;

  $: groups = jobGroups(config);
  $: rows = playerRows(config.players);
</script>

<div class="card space-y-3">
  <h2 class="text-lg font-medium">Settings</h2>
  {#if config.players.length}
    <ul class="space-y-1 text-sm">
      {#each config.players as player}
        <li>
          <span class="text-muted-foreground capitalize">{player.role}:</span>
          <a class="font-medium break-all" href="/player-configs/{player.id}">{player.name}</a>
          — {playerSummary(player)}
        </li>
      {/each}
    </ul>
  {/if}

  <details>
    <summary class="cursor-pointer text-sm font-medium">All settings</summary>
    <div class="mt-3 space-y-4">
      {#each groups as group}
        <div>
          <h3 class="mb-1 text-sm font-medium">{group.title}</h3>
          <dl class="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
            {#each group.rows as [label, value]}
              <div>
                <dt class="text-muted-foreground">{label}</dt>
                <dd class="break-all tabular-nums">{value}</dd>
              </div>
            {/each}
          </dl>
        </div>
      {/each}

      {#if config.players.length}
        <div>
          <h3 class="mb-1 text-sm font-medium">
            {config.players.length > 1 ? 'Players' : 'Player'}
          </h3>
          {#if config.players.length > 1}
            <p class="mb-1 text-xs text-muted-foreground">Settings the players differ in are in bold.</p>
          {/if}
          <div class="overflow-x-auto">
            <table class="table text-sm">
              <thead>
                <tr>
                  <th>Setting</th>
                  {#each config.players as player}
                    <th class="capitalize">{player.role}<br /><span class="font-normal break-all">{player.name}</span></th>
                  {/each}
                </tr>
              </thead>
              <tbody>
                {#each rows as row}
                  <tr class:font-semibold={row.differs}>
                    <td class="text-muted-foreground">{row.label}</td>
                    {#each row.values as value}<td class="break-all tabular-nums">{value}</td>{/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>
      {/if}

      <p class="text-xs">
        <a href="/api/jobs/{config.job.id}/config" download="job-{config.job.id}-settings.json"
          >Download every setting as JSON</a
        >
      </p>
    </div>
  </details>
</div>
