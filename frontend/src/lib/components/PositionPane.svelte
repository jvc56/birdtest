<script lang="ts">
  /**
   * One saved position: what it is, the board -- the previous move outlined,
   * the move played from here drawn where it goes -- and the moves the player
   * to move ranked, the one played marked, each with its win percentage and
   * its first two plies' statistics when it was simulated: the board beside
   * the moves on a wide screen, above them on a narrow one. Under the moves,
   * what the player inferred of the opponent's leave before simulating, when
   * it did.
   */
  import type { BoardData, SavedPosition } from '$lib/api';
  import { parseCgp, seatToMove } from '$lib/cgp';
  import { drawShare, inferenceSummary, plyAt, plyColumns, plyHeaders, showsIterations } from '$lib/moveList';
  import Board, { PLAYED_COLORS } from './Board.svelte';

  export let position: SavedPosition;
  export let board: BoardData | null;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];
  /** Before the rack in the summary line, e.g. "Game 3, turn 12". */
  export let place = '';
  /** A heading naming the position among others, e.g. "Game 1 of the pair". */
  export let heading = '';
  /** The colour the move played from here is drawn in: its player's. */
  export let playedColor: string = PLAYED_COLORS[0];

  const ANALYSIS: Record<SavedPosition['analysis'], string> = {
    static: 'static equity',
    sim: 'simulation',
    peg: 'pre-endgame solve',
    endgame: 'endgame solve'
  };

  $: parsed = position.position ? parseCgp(position.position) : null;
  $: toMove = parsed ? seatToMove(parsed, position.rack) : null;
  $: mover = toMove === null ? '' : (players[toMove] ?? '');
  $: showWinPct = position.moves.some((m) => m.win_percentage !== null);
  $: solved = position.analysis === 'peg' || position.analysis === 'endgame';
  $: plies = plyColumns(position.moves);
  $: showIters = showsIterations(position.moves);
  $: headers = plyHeaders(plies);
</script>

<div class="min-w-0 space-y-3" data-testid="saved-position" data-game-index={position.game_index}>
  {#if heading}
    <h3 class="text-sm font-medium" data-testid="position-heading">
      {heading}{#if mover}<span class="font-normal text-muted-foreground">{` · ${mover} to move`}</span>{/if}
    </h3>
  {/if}
  <p class="text-sm">
    {#if place}{place} ·{/if} rack
    <span class="font-mono" data-testid="position-rack">{position.rack}</span>
    {#if position.previous_move}
      · after <span class="font-mono">{position.previous_move}</span>
      ({position.previous_move_score})
    {/if}
    {#if position.played_move}
      · played
      <span class="font-mono" data-testid="played-move" style="text-decoration: underline dashed {playedColor}"
        >{position.played_move}</span
      >
      ({position.played_move_score})
    {/if}
    <span class="text-muted-foreground"
      >· {position.num_moves.toLocaleString()} moves ranked by
      <span data-testid="position-analysis">{ANALYSIS[position.analysis]}</span></span
    >
  </p>
  <!-- From a laptop's width up, the board and the moves share the pane
       equally, the board never under 24rem: one move list shows at a time,
       so a column fixed for the board left most of a wide screen to a table
       that needs a fraction of it. The board's own cap (Board.svelte) stops
       it growing past what a screen can read at once. The moves and the
       inference fill their column rather than leaving its right side empty,
       and scroll inside it when they are wider. Narrower, the moves go under
       the board. -->
  <div class="grid items-start gap-4 lg:grid-cols-[minmax(24rem,1fr)_minmax(0,1fr)]">
    <div class="min-w-0 space-y-2">
      {#if board && parsed}
        <Board
          {board}
          position={parsed}
          previousMove={position.previous_move}
          played={position.played_move
            ? [{ move: position.played_move, label: 'played here', color: playedColor }]
            : []}
          {toMove}
          {players}
        />
      {/if}
    </div>
    <div class="min-w-0 overflow-x-auto">
      <table class="moves table text-xs" data-testid="position-moves">
        <thead>
          <tr>
            <th>#</th><th>Move</th><th class="text-right">Score</th><th class="text-right">Equity</th>
            {#if showWinPct}<th class="text-right">Win %</th>{/if}
            {#if showIters}<th class="text-right" title="How often the simulation played the move out">Iters</th>{/if}
            {#each headers as header}<th class="text-right" title={header.title}>{header.label}</th>{/each}
            {#if solved}
              <th class="text-right" title="The mover's projected final spread">Spread</th>
              <th class="text-right" title="The endgame depth the move was ranked at">Solved Plies</th>
            {/if}
          </tr>
        </thead>
        <tbody>
          {#each position.moves as move}
            {@const playedHere = move.move === position.played_move}
            <tr class:played-row={playedHere} data-played={playedHere || undefined}>
              <td class="tabular-nums">{move.rank}</td>
              <td class="font-mono">
                {move.move}{#if playedHere}<span
                    class="ml-1 rounded px-1 font-sans text-[10px] uppercase"
                    style="background: {playedColor}; color: white">played</span
                  >{/if}
              </td>
              <td class="text-right tabular-nums">{move.score}</td>
              <td class="text-right tabular-nums">{move.equity.toFixed(2)}</td>
              {#if showWinPct}
                <td class="text-right tabular-nums">
                  {move.win_percentage === null ? '—' : move.win_percentage.toFixed(1)}
                </td>
              {/if}
              {#if showIters}
                <td class="text-right tabular-nums">{move.iterations ? move.iterations.toLocaleString() : '—'}</td>
              {/if}
              {#each Array(plies) as _, i}
                {@const stats = plyAt(move.plies, i)}
                <td class="text-right tabular-nums">{stats ? stats.average_score.toFixed(1) : '—'}</td>
                <td class="text-right tabular-nums">{stats ? `${stats.bingo_percentage.toFixed(1)}%` : '—'}</td>
              {/each}
              {#if solved}
                <td class="text-right tabular-nums">
                  {move.mean_spread === null ? '—' : move.mean_spread.toFixed(1)}
                </td>
                <td class="text-right tabular-nums">{move.fidelity_plies ?? '—'}</td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
      {#if position.inference}
        {@const inference = position.inference}
        <div class="mt-3 space-y-1" data-testid="position-inference">
          <p class="text-xs">{inferenceSummary(inference, position.previous_move)}.</p>
          {#if inference.leaves.length}
            <!-- The column's width, like the moves above it. -->
            <table class="moves table text-xs">
              <thead>
                <tr>
                  <th title="What the opponent kept">Leave</th>
                  <th class="text-right">Draws</th>
                  <th class="text-right">Equity</th>
                </tr>
              </thead>
              <tbody>
                {#each inference.leaves as leave}
                  <tr>
                    <td class="font-mono">{leave.leave || '—'}</td>
                    <td class="text-right tabular-nums">
                      {leave.draws.toLocaleString()} ({drawShare(leave.draws, inference.total_draws)})
                    </td>
                    <td class="text-right tabular-nums">{leave.equity.toFixed(1)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  /* One line a cell, and room for nine columns beside the board. */
  .moves :global(th),
  .moves :global(td) {
    white-space: nowrap;
    padding-left: 0.3rem;
    padding-right: 0.3rem;
  }
  .played-row {
    background: hsl(140 45% 50% / 0.12);
  }
</style>
