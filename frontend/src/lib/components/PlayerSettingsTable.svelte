<script lang="ts">
  /**
   * Player configs' settings as a table: a column per player -- one on a
   * config's own page or a leave job's, two side by side for a games or pairs
   * job -- with the rows the players differ in bold. The key rows unless
   * `all`, when every setting. The card around it holds the toggle, so one
   * toggle can open a job's settings and its players' together.
   */
  import { keySettings, playerRows, type PlayerSettings } from '$lib/jobSettings';

  export let players: PlayerSettings[];
  /** Every setting rather than the key rows. */
  export let all = false;

  $: rows = all ? playerRows(players) : keySettings(players);
  // A job's players are headed by their part in it and link to their pages;
  // a config read on its own has no part, and its page is its heading.
  $: headed = players.some((p) => p.role);
  $: compared = players.length > 1 && rows.some((r) => r.differs);
</script>

<div class="space-y-1">
  {#if compared}
    <p class="text-xs text-muted-foreground">Settings the players differ in are in bold.</p>
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
          <tr class:font-semibold={row.differs}>
            <td class="w-1/3 text-muted-foreground">{row.label}</td>
            {#each row.values as value}<td class="break-words tabular-nums">{value}</td>{/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>
