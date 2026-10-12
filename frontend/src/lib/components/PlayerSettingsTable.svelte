<script lang="ts">
  /**
   * Player configs' settings as a table: a column per player -- one on a
   * config's own page or a leave job's, two side by side for a games or pairs
   * job. It lists, by `mode`, the settings the players differ in, the key
   * rows, or every one; the card around it holds the toggle. Two players
   * alike have no differences, and say so rather than show an empty table.
   * Settings the job never reads (`unused`, by row id) are shown muted, and
   * are never a difference. Listed among settings they share, the
   * ones two players differ in are marked: an amber edge and tint, the label
   * and values in weight; listed alone they are the table, and unmarked. A
   * job's players are headed by their
   * part in it, their colour -- the one their moves are drawn in on a saved
   * position's board -- and their name, which links to the config's page.
   */
  import { settingsFor, type PlayerSettings, type SettingsMode } from '$lib/jobSettings';
  import { PLAYED_COLORS } from './Board.svelte';

  export let players: PlayerSettings[];
  export let mode: SettingsMode = 'key';
  /** Row ids of the settings the job never reads. */
  export let unused: ReadonlySet<string> = new Set();

  $: rows = settingsFor(players, mode, unused);
  // A job's players are headed by their part in it and link to their pages;
  // a config read on its own has no part, and its page is its heading.
  $: headed = players.some((p) => p.role);
  // Two players, each in the colour the saved positions draw their moves in.
  $: colored = players.length === PLAYED_COLORS.length;
  // Only beside settings the players share does a difference need marking.
  $: marking = players.length > 1 && mode !== 'differences';
</script>

<div class="space-y-2">
  {#if mode === 'differences' && !rows.length}
    <p class="text-sm text-muted-foreground" data-testid="settings-identical">
      These players' settings are identical.
    </p>
  {:else}
    <!-- Values wrap between words, and a table still too wide for a phone
         scrolls inside its box rather than widening the page. A name may be
         32 characters with no break, so it may break anywhere. -->
    <div class="overflow-x-auto">
      <table class="table text-sm">
        {#if headed}
          <thead>
            <tr>
              <th class="w-1/3">{players.length > 1 ? 'Players' : 'Player'}</th>
              {#each players as player, i}
                <th>
                  <span class="inline-flex items-center gap-1.5">
                    {#if colored}
                      <span
                        class="inline-block h-2.5 w-2.5 shrink-0 rounded-full"
                        style="background: {PLAYED_COLORS[i]}"
                        data-testid="player-color"
                      ></span>
                    {/if}
                    <span class="capitalize">{player.role}</span>
                  </span><br />
                  <a class="font-normal [overflow-wrap:anywhere]" href="/player-configs/{player.id}">{player.name}</a>
                </th>
              {/each}
            </tr>
          </thead>
        {/if}
        <tbody data-testid="setting-rows">
          {#each rows as row}
            {@const marked = marking && row.differs}
            <tr
              class="{row.unused ? 'text-muted-foreground' : ''} {marked ? 'bg-warning/10' : ''}"
              data-setting={row.id}
              data-differs={marked || undefined}
            >
              <td
                class="w-1/3 {marked
                  ? 'font-medium text-foreground shadow-[inset_3px_0_0_hsl(38_92%_50%)]'
                  : 'text-muted-foreground'}"
              >
                {row.label}{#if marked}<span class="sr-only"> (the players differ)</span>{/if}
              </td>
              {#each row.values as value}<td class="break-words tabular-nums" class:font-semibold={marked}>{value}</td>{/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>
