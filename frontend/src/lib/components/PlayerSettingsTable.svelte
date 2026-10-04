<script lang="ts">
  /**
   * Player configs' settings as a table: a column per player -- one on a
   * config's own page or a leave job's, two side by side for a games or pairs
   * job -- with the rows the players differ in bold. The key rows unless
   * `all`, when every setting; the card around it holds the toggle. Settings the
   * job never reads (`unused`, by row label) are shown muted, with a note. A
   * job's players are headed by their part in it and their name, which links
   * to the config's page.
   */
  import { keySettings, playerRows, type PlayerSettings } from '$lib/jobSettings';

  export let players: PlayerSettings[];
  /** Every setting rather than the key rows. */
  export let all = false;
  /** Row labels of the settings the job never reads. */
  export let unused: ReadonlySet<string> = new Set();

  $: rows = all ? playerRows(players, unused) : keySettings(players, unused);
  $: muted = rows.some((r) => r.unused);
  // A job's players are headed by their part in it and link to their pages;
  // a config read on its own has no part, and its page is its heading.
  $: headed = players.some((p) => p.role);
  $: compared = players.length > 1 && rows.some((r) => r.differs);
</script>

<div class="space-y-1">
  {#if compared}
    <p class="text-xs text-muted-foreground">Settings the players differ in are in bold.</p>
  {/if}
  {#if muted}
    <p class="text-xs text-muted-foreground">
      Settings in grey are the player config's, and this job does not use them.
    </p>
  {/if}
  <!-- Values wrap between words, and a table still too wide for a phone
       scrolls inside its box rather than widening the page. A name may be
       32 characters with no break, so it may break anywhere. -->
  <div class="overflow-x-auto">
    <table class="table text-sm">
      {#if headed}
        <thead>
          <tr>
            <th class="w-1/3">{players.length > 1 ? 'Players' : 'Player'}</th>
            {#each players as player}
              <th>
                <span class="capitalize">{player.role}</span><br />
                <a class="font-normal [overflow-wrap:anywhere]" href="/player-configs/{player.id}">{player.name}</a>
              </th>
            {/each}
          </tr>
        </thead>
      {/if}
      <tbody>
        {#each rows as row}
          <tr class:font-semibold={row.differs} class:text-muted-foreground={row.unused}>
            <td class="w-1/3 text-muted-foreground">{row.label}</td>
            {#each row.values as value}<td class="break-words tabular-nums">{value}</td>{/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>
