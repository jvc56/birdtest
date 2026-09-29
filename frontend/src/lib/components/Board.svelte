<script lang="ts">
  /**
   * A saved position on the job's own board: the premium squares from its
   * layout, the tiles with their letters and scores (a blank in lower case,
   * in red, scoring nothing), the tiles the previous move placed outlined,
   * and below it both racks and scores, the player to move marked.
   *
   * One SVG in board units, so it scales to its box: at phone width the
   * board shrinks rather than pushing the page sideways.
   */
  import type { BoardData, BoardSquare } from '$lib/api';
  import { placedSquares, type Position, type Tile } from '$lib/cgp';

  export let board: BoardData;
  export let position: Position;
  /** The move that led here, as MAGPIE names it. */
  export let previousMove: string | null = null;
  /** The seat to move, when known. */
  export let toMove: 0 | 1 | null = null;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  const PREMIUM: Record<BoardSquare, { fill: string; label: string; name: string }> = {
    normal: { fill: 'hsl(217 19% 17%)', label: '', name: '' },
    double_letter: { fill: 'hsl(195 55% 32%)', label: 'DL', name: 'double letter' },
    triple_letter: { fill: 'hsl(220 60% 38%)', label: 'TL', name: 'triple letter' },
    quadruple_letter: { fill: 'hsl(265 45% 40%)', label: 'QL', name: 'quadruple letter' },
    double_word: { fill: 'hsl(330 45% 38%)', label: 'DW', name: 'double word' },
    triple_word: { fill: 'hsl(0 58% 40%)', label: 'TW', name: 'triple word' },
    quadruple_word: { fill: 'hsl(30 70% 38%)', label: 'QW', name: 'quadruple word' },
    brick: { fill: 'hsl(217 10% 32%)', label: '', name: 'brick' }
  };
  // Units to a square. Whole numbers rather than one a square: a browser may
  // hold text to a minimum size, and a letter 0.6 units tall is below any.
  const U = 100;
  // The board's margin, for the column letters and row numbers.
  const M = 0.7 * U;

  $: dim = board.squares.length;
  $: scores = new Map(board.letters.map((l) => [l.letter, l.score]));
  $: placed = placedSquares(previousMove);
  $: used = new Set(board.squares.flat());
  $: legend = (Object.keys(PREMIUM) as BoardSquare[]).filter((s) => s !== 'normal' && used.has(s));
  $: names = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'];
  $: tileCount = position.board.flat().filter(Boolean).length;

  /** What a tile scores: a blank nothing, a letter the board data does not know unmarked. */
  function score(tile: Tile): number | undefined {
    return tile.blank ? undefined : scores.get(tile.letter);
  }

  /** The letter's size: a multi-character tile (`L·L`) is set smaller to fit its square. */
  function fontSize(letter: string): number {
    const length = Array.from(letter).length;
    return (length > 1 ? 1.1 / (length + 0.6) : 0.6) * U;
  }
</script>

<div class="space-y-3">
  <svg
    viewBox="{-M} {-M} {dim * U + M} {dim * U + M}"
    class="block h-auto w-full max-w-xl select-none"
    role="img"
    aria-label="The board: {tileCount} tiles{placed.size ? ', the previous move outlined' : ''}"
    data-testid="board"
  >
    {#each Array(dim) as _, i}
      <text x={(i + 0.5) * U} y={-0.2 * U} class="coord" text-anchor="middle">{String.fromCharCode(65 + i)}</text>
      <text x={-0.35 * U} y={(i + 0.62) * U} class="coord" text-anchor="middle">{i + 1}</text>
    {/each}
    {#each board.squares as row, r}
      {#each row as square, c}
        {@const tile = position.board[r]?.[c]}
        {#if tile}
          {@const last = placed.has(`${r},${c}`)}
          {@const points = score(tile)}
          <g class="tile" class:last data-letter={tile.letter} data-blank={tile.blank || undefined}>
            <rect
              x={(c + 0.04) * U}
              y={(r + 0.04) * U}
              width={0.92 * U}
              height={0.92 * U}
              rx={0.1 * U}
              fill={last ? 'hsl(45 90% 72%)' : 'hsl(40 55% 80%)'}
              stroke={last ? 'hsl(199 89% 48%)' : 'none'}
              stroke-width={0.1 * U}
            />
            <text
              x={(c + (points === undefined ? 0.5 : 0.45)) * U}
              y={(r + 0.53) * U}
              font-size={fontSize(tile.letter)}
              text-anchor="middle"
              dominant-baseline="central"
              class="letter"
              fill={tile.blank ? 'hsl(0 70% 42%)' : 'hsl(222 47% 10%)'}
            >{tile.blank ? tile.letter.toLowerCase() : tile.letter}</text>
            {#if points !== undefined}
              <text x={(c + 0.88) * U} y={(r + 0.86) * U} font-size={0.26 * U} text-anchor="end" fill="hsl(222 47% 10%)"
                >{points}</text
              >
            {/if}
          </g>
        {:else}
          <rect
            x={(c + 0.04) * U}
            y={(r + 0.04) * U}
            width={0.92 * U}
            height={0.92 * U}
            rx={0.08 * U}
            fill={PREMIUM[square].fill}
          ><title>{PREMIUM[square].name}</title></rect>
          {#if r === board.start[0] && c === board.start[1]}
            <text x={(c + 0.5) * U} y={(r + 0.56) * U} font-size={0.55 * U} text-anchor="middle" dominant-baseline="central" class="mark"
              >★</text
            >
          {:else if PREMIUM[square].label}
            <text x={(c + 0.5) * U} y={(r + 0.53) * U} font-size={0.3 * U} text-anchor="middle" dominant-baseline="central" class="mark"
              >{PREMIUM[square].label}</text
            >
          {/if}
        {/if}
      {/each}
    {/each}
  </svg>

  <div class="grid gap-2 sm:grid-cols-2">
    {#each position.racks as rack, seat}
      <div class="space-y-1" data-testid="rack">
        <p class="flex flex-wrap items-baseline gap-x-2 text-sm">
          <span class="text-muted-foreground">Player {seat + 1}</span>
          <span class="break-all">{names[seat]}</span>
          <span class="tabular-nums font-medium" data-testid="score">{position.scores[seat]}</span>
          {#if toMove === seat}
            <span class="rounded-full bg-primary px-2 text-xs text-primary-foreground">to move</span>
          {/if}
        </p>
        <div class="flex min-h-8 flex-wrap gap-1">
          {#each rack as tile}
            {@const points = score(tile)}
            <span
              class="rack-tile"
              class:blank={tile.blank}
              data-letter={tile.letter}
              title={tile.blank ? 'blank' : undefined}
            >
              <span class="rack-letter" class:long={Array.from(tile.letter).length > 1}>{tile.letter}</span>
              {#if points !== undefined}<span class="rack-score">{points}</span>{/if}
            </span>
          {:else}
            <span class="text-sm text-muted-foreground">empty rack</span>
          {/each}
        </div>
      </div>
    {/each}
  </div>

  {#if legend.length}
    <p class="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
      {#each legend as square}
        <span class="inline-flex items-center gap-1">
          <span class="inline-block h-3 w-3 rounded-sm" style="background: {PREMIUM[square].fill}"></span>
          {PREMIUM[square].name}
        </span>
      {/each}
      {#if placed.size}
        <span class="inline-flex items-center gap-1">
          <span class="inline-block h-3 w-3 rounded-sm border-2 border-primary" style="background: hsl(45 90% 72%)"
          ></span>
          previous move
        </span>
      {/if}
    </p>
  {/if}
</div>

<style>
  .coord {
    font-size: 30px;
    fill: hsl(215 14% 60%);
  }
  .mark {
    fill: hsl(210 20% 92% / 0.8);
  }
  .letter {
    font-weight: 700;
  }
  .rack-tile {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: 0.25rem;
    background: hsl(40 55% 80%);
    color: hsl(222 47% 10%);
    font-weight: 700;
  }
  .rack-tile.blank {
    color: hsl(0 70% 42%);
  }
  .rack-letter.long {
    font-size: 0.65rem;
  }
  .rack-score {
    position: absolute;
    right: 0.15rem;
    bottom: 0;
    font-size: 0.55rem;
    font-weight: 400;
  }
</style>
