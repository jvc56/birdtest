<script lang="ts">
  /**
   * One saved position: what it is, the board -- the previous move outlined,
   * the move played from here drawn where it goes -- and the moves the player
   * to move ranked, the one played marked: the board beside the moves on a
   * wide screen, above them on a narrow one.
   */
  import type { BoardData, SavedPosition } from '$lib/api';
  import { parseCgp, seatToMove } from '$lib/cgp';
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
</script>

<div class="min-w-0 space-y-3" data-testid="saved-position" data-game-index={position.game_index}>
  {#if heading}
    <h3 class="text-sm font-medium" data-testid="position-heading">
      {heading}{#if mover}<span class="font-normal text-muted-foreground"> · {mover} to move</span>{/if}
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
  <div class="grid gap-4 lg:grid-cols-2">
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
      <table class="table text-xs">
        <thead>
          <tr>
            <th>#</th><th>Move</th><th class="text-right">Score</th><th class="text-right">Equity</th>
            {#if showWinPct}<th class="text-right">Win %</th>{/if}
            {#if solved}
              <th class="text-right" title="The mover's projected final spread">Spread</th>
              <th class="text-right" title="The endgame depth the move was ranked at">Plies</th>
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
    </div>
  </div>
</div>

<style>
  .played-row {
    background: hsl(140 45% 50% / 0.12);
  }
</style>
