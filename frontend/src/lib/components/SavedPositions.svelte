<script lang="ts">
  /**
   * The positions a games or pairs job captured (`capture_positions`), one at
   * a time on the job's board: a random one, or those where the player to
   * move held one rack, newest first. Signed-in users only -- the routes
   * refuse anyone else.
   */
  import { onMount } from 'svelte';
  import { api, errorText, type BoardData, type SavedPosition } from '$lib/api';
  import { parseCgp, seatToMove } from '$lib/cgp';
  import Board from './Board.svelte';

  export let jobId: string;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  let board: BoardData | null = null;
  let position: SavedPosition | null = null;
  let loaded = false;
  let busy = false;
  let error = '';

  let rackQuery = '';
  // The rack search on screen, if any: the rack as typed, the cursor of each
  // position shown so far (the first is none), where in them it is, and the
  // cursor of the next one if there is one.
  let search: { rack: string; cursors: (string | undefined)[]; at: number; next?: string } | null = null;

  async function run(load: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await load();
      loaded = true;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  const random = () =>
    run(async () => {
      position = await api.randomPosition(jobId);
      search = null;
    });

  /** The `at`th position of `rack`'s search, reached through `cursors`. */
  const searchAt = (rack: string, cursors: (string | undefined)[], at: number) =>
    run(async () => {
      const page = await api.jobPositions(jobId, rack, { per_page: 1, cursor: cursors[at] });
      position = page.items[0] ?? null;
      search = { rack, cursors, at, next: page.next_cursor };
    });

  function find() {
    const rack = rackQuery.trim();
    if (rack) searchAt(rack, [undefined], 0);
  }

  onMount(() => {
    // The board is only drawn with its data; a position without it still
    // shows as text.
    api.jobBoard(jobId).then(
      (b) => (board = b),
      () => (board = null)
    );
    random();
  });

  $: parsed = position?.position ? parseCgp(position.position) : null;
  $: toMove = parsed && position ? seatToMove(parsed, position.rack) : null;
  $: showWinPct = position?.moves.some((m) => m.win_percentage !== null) ?? false;
</script>

<div class="card space-y-4">
  <div class="space-y-1">
    <h2 class="text-lg font-medium">Saved positions</h2>
    <p class="text-sm text-muted-foreground">
      This job keeps the position analysed on every turn of every game: the board, both racks, and
      the moves the player to move ranked.
    </p>
  </div>

  <div class="flex flex-wrap gap-2">
    <button class="btn-primary" disabled={busy} on:click={random}>Random position</button>
    <label class="sr-only" for="position-rack">Rack</label>
    <input
      id="position-rack"
      class="input w-40"
      bind:value={rackQuery}
      placeholder="Rack, e.g. AEINRS?"
      on:keydown={(e) => e.key === 'Enter' && find()}
    />
    <button class="btn-secondary" disabled={busy} on:click={find}>Search</button>
  </div>

  {#if error}<p class="field-error" role="alert">{error}</p>{/if}

  {#if loaded && !position}
    <p class="text-sm text-muted-foreground">
      {search ? `No saved position has the rack ${search.rack.toUpperCase()}.` : 'No positions saved yet.'}
    </p>
  {/if}

  {#if search && position}
    <div class="flex flex-wrap items-center gap-2 text-sm" data-testid="rack-search">
      <span class="text-muted-foreground">
        <!-- The rack as the server spells it (the job's letter order, the blank
             last), which the position shown holds, rather than as it was typed. -->
        Position {search.at + 1} with the rack <span class="font-mono">{position.rack}</span>, newest first
      </span>
      {#if search.at > 0}
        <button
          class="btn-secondary"
          disabled={busy}
          on:click={() => search && searchAt(search.rack, search.cursors, search.at - 1)}>Previous</button
        >
      {/if}
      {#if search.next}
        <button
          class="btn-secondary"
          disabled={busy}
          on:click={() =>
            search &&
            searchAt(search.rack, [...search.cursors.slice(0, search.at + 1), search.next], search.at + 1)}
          >Next</button
        >
      {/if}
    </div>
  {/if}

  {#if position}
    <div class="space-y-3" data-testid="saved-position">
      <p class="text-sm">
        Game {position.game_index + 1}, turn {position.turn_number + 1} · rack
        <span class="font-mono" data-testid="position-rack">{position.rack}</span>
        {#if position.previous_move}
          · after <span class="font-mono">{position.previous_move}</span>
          ({position.previous_move_score})
        {/if}
        <span class="text-muted-foreground">· {position.num_moves.toLocaleString()} moves ranked</span>
      </p>
      <div class="grid gap-4 lg:grid-cols-2">
        <div class="min-w-0 space-y-2">
          {#if board && parsed}
            <Board {board} position={parsed} previousMove={position.previous_move} {toMove} {players} />
          {/if}
          {#if position.position}
            <p class="break-all font-mono text-xs text-muted-foreground" title="CGP">{position.position}</p>
          {/if}
        </div>
        <div class="min-w-0 overflow-x-auto">
          <table class="table text-xs">
            <thead>
              <tr>
                <th>#</th><th>Move</th><th class="text-right">Score</th><th class="text-right">Equity</th>
                {#if showWinPct}<th class="text-right">Win %</th>{/if}
              </tr>
            </thead>
            <tbody>
              {#each position.moves as move}
                <tr>
                  <td class="tabular-nums">{move.rank}</td>
                  <td class="font-mono">{move.move}</td>
                  <td class="text-right tabular-nums">{move.score}</td>
                  <td class="text-right tabular-nums">{move.equity.toFixed(2)}</td>
                  {#if showWinPct}
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
    </div>
  {/if}
</div>
