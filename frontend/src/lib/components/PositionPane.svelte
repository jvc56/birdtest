<script lang="ts">
  /**
   * One saved position: what it is, the board, its CGP and the moves the
   * player to move ranked. Beside the moves on a wide screen, or above them
   * when `stacked` -- as each of a game pair's two positions is, side by side.
   */
  import type { BoardData, SavedPosition } from '$lib/api';
  import { parseCgp, seatToMove } from '$lib/cgp';
  import Board from './Board.svelte';

  export let position: SavedPosition;
  export let board: BoardData | null;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];
  /** Before the rack in the summary line, e.g. "Game 3, turn 12". */
  export let place = '';
  /** A heading naming the position among others, e.g. "Game 1 of the pair". */
  export let heading = '';
  export let stacked = false;

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
    <span class="text-muted-foreground"
      >· {position.num_moves.toLocaleString()} moves ranked by
      <span data-testid="position-analysis">{ANALYSIS[position.analysis]}</span></span
    >
  </p>
  <div class={stacked ? 'grid gap-4' : 'grid gap-4 lg:grid-cols-2'}>
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
            {#if solved}
              <th class="text-right" title="The mover's projected final spread">Spread</th>
              <th class="text-right" title="The endgame depth the move was ranked at">Plies</th>
            {/if}
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
