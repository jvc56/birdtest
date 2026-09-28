<script lang="ts">
  /**
   * The positions a games or pairs job captured (`capture_positions`): the
   * latest first, or those where the player to move held one rack. Signed-in
   * users only -- the route refuses anyone else.
   */
  import { onMount } from 'svelte';
  import { api, errorText, type SavedPosition } from '$lib/api';

  export let jobId: string;

  const PER_PAGE = 10;

  let rackQuery = '';
  // The rack the list on screen is for: '' is every position.
  let shownRack = '';
  let positions: SavedPosition[] = [];
  let nextCursor: string | undefined;
  let loaded = false;
  let busy = false;
  let error = '';

  async function load(rack: string, cursor?: string) {
    busy = true;
    error = '';
    try {
      const page = await api.jobPositions(jobId, {
        per_page: PER_PAGE,
        ...(rack ? { rack } : {}),
        ...(cursor ? { cursor } : {})
      });
      positions = cursor ? [...positions, ...page.items] : page.items;
      nextCursor = page.next_cursor;
      shownRack = rack;
      loaded = true;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  onMount(() => load(''));
</script>

<div class="card space-y-4">
  <div class="space-y-1">
    <h2 class="text-lg font-medium">Saved positions</h2>
    <p class="text-sm text-muted-foreground">
      This job keeps the position analysed on every turn of every game: the board, the rack of
      the player to move, and the moves they ranked.
    </p>
  </div>

  <div class="flex flex-wrap gap-2">
    <label class="sr-only" for="position-rack">Rack</label>
    <input
      id="position-rack"
      class="input max-w-xs"
      bind:value={rackQuery}
      placeholder="Rack, e.g. AEINRST"
      on:keydown={(e) => e.key === 'Enter' && load(rackQuery.trim())}
    />
    <button class="btn-primary" disabled={busy} on:click={() => load(rackQuery.trim())}>
      Search
    </button>
    {#if shownRack}
      <button
        class="btn-secondary"
        disabled={busy}
        on:click={() => {
          rackQuery = '';
          load('');
        }}>Show all</button
      >
    {/if}
  </div>

  {#if error}<p class="field-error" role="alert">{error}</p>{/if}

  {#if loaded && !positions.length}
    <p class="text-sm text-muted-foreground">
      {shownRack ? `No saved position has the rack ${shownRack.toUpperCase()}.` : 'No positions saved yet.'}
    </p>
  {/if}

  {#each positions as position}
    <div class="space-y-2 border-t border-border pt-3">
      <p class="text-sm">
        Game {position.game_index + 1}, turn {position.turn_number + 1} · rack
        <span class="font-mono">{position.rack}</span>
        {#if position.previous_move}
          · after <span class="font-mono">{position.previous_move}</span>
          ({position.previous_move_score})
        {/if}
        <span class="text-muted-foreground">· {position.num_moves.toLocaleString()} moves ranked</span>
      </p>
      {#if position.position}
        <p class="break-all font-mono text-xs text-muted-foreground" title="CGP">{position.position}</p>
      {/if}
      <div class="overflow-x-auto">
        <table class="table text-xs">
          <thead>
            <tr>
              <th>#</th><th>Move</th><th class="text-right">Score</th><th class="text-right">Equity</th>
              {#if position.moves.some((m) => m.win_percentage !== null)}<th class="text-right">Win %</th>{/if}
            </tr>
          </thead>
          <tbody>
            {#each position.moves as move}
              <tr>
                <td class="tabular-nums">{move.rank}</td>
                <td class="font-mono">{move.move}</td>
                <td class="text-right tabular-nums">{move.score}</td>
                <td class="text-right tabular-nums">{move.equity.toFixed(2)}</td>
                {#if position.moves.some((m) => m.win_percentage !== null)}
                  <td class="text-right tabular-nums">
                    {move.win_percentage === null ? '—' : move.win_percentage.toFixed(1)}
                  </td>
                {/if}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/each}

  {#if nextCursor}
    <button class="btn-secondary" disabled={busy} on:click={() => load(shownRack, nextCursor)}>
      {busy ? 'Loading…' : 'Load more'}
    </button>
  {/if}
</div>
