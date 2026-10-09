<script lang="ts">
  /**
   * Player configs' settings as a table: a column per player -- one on a
   * config's own page or a leave job's, two side by side for a games or pairs
   * job -- differences first. What the players differ in is a block of its
   * own at the top, tinted, from every setting; what they share follows,
   * folded away behind "Show N shared settings" while there are differences
   * to read (one player, or two alike, shows it open). The shared rows are
   * the key ones unless `all`, when every setting; the card around it holds
   * that toggle, and turning it on opens them. Settings the job never reads
   * (`unused`, by row id) are shown muted, with a note, and are never a
   * difference. A job's players are headed by their part in it, their colour
   * -- the one their moves are drawn in on a saved position's board -- and
   * their name, which links to the config's page.
   */
  import { settingBlocks, type PlayerSettings } from '$lib/jobSettings';
  import { PLAYED_COLORS } from './Board.svelte';

  export let players: PlayerSettings[];
  /** Every setting rather than the key rows. */
  export let all = false;
  /** Row ids of the settings the job never reads. */
  export let unused: ReadonlySet<string> = new Set();

  $: ({ differences, shared } = settingBlocks(players, all, unused));
  // A job's players are headed by their part in it and link to their pages;
  // a config read on its own has no part, and its page is its heading.
  $: headed = players.some((p) => p.role);
  // Two players, each in the colour the saved positions draw their moves in.
  $: colored = players.length === PLAYED_COLORS.length;

  let showShared = false;
  // Asking for every setting is asking to see them.
  $: if (all) showShared = true;
  $: folded = differences.length > 0 && !showShared;
  $: muted = [...differences, ...(folded ? [] : shared)].some((r) => r.unused);
</script>

<div class="space-y-2">
  {#if muted}
    <p class="text-xs text-muted-foreground">
      Settings in grey are the player config's, and this job does not use them.
    </p>
  {/if}
  <!-- Values wrap between words, and a table still too wide for a phone
       scrolls inside its box rather than widening the page. A name may be
       32 characters with no break, so it may break anywhere. One table for
       both blocks, so their columns line up. The accent's colour is
       important: `.table`'s cells set every border's colour, and a selector
       with the element in it outranks a utility's. -->
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
      {#if differences.length}
        <tbody data-testid="setting-differences">
          <tr>
            <th
              colspan={players.length + 1}
              scope="rowgroup"
              class="border-l-4 !border-l-warning bg-warning/10 text-xs font-semibold uppercase tracking-wide !text-warning"
            >
              Differences ({differences.length})
            </th>
          </tr>
          {#each differences as row}
            <tr class="bg-warning/10" data-setting={row.id}>
              <td class="w-1/3 border-l-4 !border-l-warning text-muted-foreground">{row.label}</td>
              {#each row.values as value}<td class="break-words font-medium tabular-nums">{value}</td>{/each}
            </tr>
          {/each}
        </tbody>
      {/if}
      {#if !folded}
        <tbody data-testid="shared-settings">
          {#each shared as row}
            <tr class:text-muted-foreground={row.unused} data-setting={row.id}>
              <td class="w-1/3 text-muted-foreground">{row.label}</td>
              {#each row.values as value}<td class="break-words tabular-nums">{value}</td>{/each}
            </tr>
          {/each}
        </tbody>
      {/if}
    </table>
  </div>
  {#if differences.length && shared.length}
    <button
      type="button"
      class="text-sm text-primary hover:underline"
      aria-expanded={!folded}
      on:click={() => (showShared = !showShared)}
    >
      {folded
        ? `Show ${shared.length} shared setting${shared.length === 1 ? '' : 's'}`
        : 'Hide shared settings'}
    </button>
  {/if}
</div>
