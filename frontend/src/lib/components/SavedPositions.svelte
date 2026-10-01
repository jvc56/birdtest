<script lang="ts">
  /**
   * The positions a games or pairs job captured (`capture_positions`), one at
   * a time on the job's board: a random one, or those where the player to
   * move held one rack, newest first. Signed-in users only -- the routes
   * refuse anyone else.
   *
   * A pairs job's position comes with its partner, the same turn of the
   * pair's other game, and the two are shown side by side: up to the turn a
   * pair's games diverge they are one game with the seats swapped, so the two
   * are each player's answer to one position. A job keeping only first
   * divergences keeps exactly those.
   */
  import { onMount } from 'svelte';
  import { api, errorText, type BoardData, type SavedPosition } from '$lib/api';
  import PositionPane from './PositionPane.svelte';

  export let jobId: string;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];
  /** A game-pairs job's: each position is shown beside its partner. */
  export let paired = false;
  /** A pairs job that keeps only each pair's first divergence. */
  export let firstDivergence = false;

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

  // A pair's positions, its first game's first: the one found and its
  // partner, or the one alone when the other game has none at that turn.
  $: pair =
    paired && position
      ? [position, position.partner ?? null]
          .filter((p): p is SavedPosition => p !== null)
          .sort((a, b) => a.game_index - b.game_index)
      : [];
</script>

<div class="card space-y-4">
  <div class="space-y-1">
    <h2 class="text-lg font-medium">Saved positions</h2>
    <p class="text-sm text-muted-foreground">
      {#if paired && firstDivergence}
        This job keeps, from each game pair, the turn where its two games first diverged: the one
        position both players faced, once from each seat, and the moves each ranked. A pair played
        identically keeps nothing.
      {:else if paired}
        This job keeps the position analysed on every turn of every game: the board, both racks, and
        the moves the player to move ranked. A pair's two games are shown side by side at one turn.
      {:else}
        This job keeps the position analysed on every turn of every game: the board, both racks, and
        the moves the player to move ranked.
      {/if}
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

  {#if position && paired}
    <div class="space-y-3" data-testid="saved-pair">
      <p class="text-sm">
        Turn {position.turn_number + 1} of a game pair{#if firstDivergence}, where the players first
          chose differently{/if}
      </p>
      <div class="grid gap-6 lg:grid-cols-2">
        {#each pair as game (game.game_index)}
          <PositionPane
            position={game}
            {board}
            {players}
            stacked
            heading={`Game ${(game.game_index % 2) + 1} of the pair`}
          />
        {/each}
      </div>
      {#if pair.length === 1}
        <p class="text-sm text-muted-foreground">
          The pair's other game has no position at this turn: it had ended, or its result is not in.
        </p>
      {/if}
    </div>
  {:else if position}
    <PositionPane
      {position}
      {board}
      {players}
      place={`Game ${position.game_index + 1}, turn ${position.turn_number + 1}`}
    />
  {/if}
</div>
