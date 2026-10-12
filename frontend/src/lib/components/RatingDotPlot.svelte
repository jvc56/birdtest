<script lang="ts">
  /**
   * Ratings as a dot plot with error bars, each row's rating and error printed
   * at its end.
   *
   * Deliberately **not** a bar chart. A bar encodes magnitude from zero, and
   * a rating has no meaningful zero — only gaps carry meaning (WESPA's scale,
   * 250 points a logit), and the level is wherever the pool's anchor was
   * pinned — so bar length would imply a ratio that does not exist.
   * A dot on a common scale encodes position, which is what a rating is.
   *
   * The error bar is the point of the chart as much as the dot: a config with
   * few pairs and one with millions produce identical-looking numbers, and only
   * the interval says which one to believe. Each has two widths: a thick ±1
   * standard error and a thin 95% interval, the one to read two configs'
   * difference by.
   */
  import type { RatingRow } from '$lib/api';
  import {
    dotPlotX,
    dotTitle,
    layoutDotPlot,
    NARROW_NAME_CHARS,
    NARROW_WIDTH,
    NAME_CHARS,
    ratingFigure,
    rowLabels
  } from '$lib/charts/ratingDotPlot';

  export let ratings: RatingRow[] = [];

  let width = 720;

  // Unrated configs have no position on the scale, so they are listed beneath
  // the chart rather than drawn at a number that means nothing. Runaway error
  // bars are clamped (see visibleError); the printed error is the real one.
  $: layout = layoutDotPlot(ratings, width);
  $: ({ rated, unrated, bounds, ticks, tickStep, height, pad } = layout);
  $: labels = rowLabels(
    rated.map((r) => r.row),
    width < NARROW_WIDTH ? NARROW_NAME_CHARS : NAME_CHARS
  );
  $: x = (value: number) => dotPlotX(value, bounds, width);
</script>

<div class="w-full" bind:clientWidth={width}>
  {#if rated.length}
    <div class="mb-2 flex flex-wrap items-center gap-x-5 gap-y-1 text-xs text-muted-foreground">
      <span class="inline-flex items-center gap-1.5">
        <svg width="44" height="10" aria-hidden="true">
          <line x1="2" x2="42" y1="5" y2="5" stroke="hsl(215 14% 60%)" stroke-width="1.5" stroke-linecap="round" />
          <line x1="13" x2="31" y1="5" y2="5" stroke="hsl(215 14% 75%)" stroke-width="4" stroke-linecap="round" />
          <circle cx="22" cy="5" r="4" fill="hsl(199 89% 48%)" />
        </svg>
        thick: ±1 standard error · thin: 95% interval
      </span>
      <span class="inline-flex items-center gap-1.5">
        <svg width="10" height="10" aria-hidden="true"><circle cx="5" cy="5" r="4.5" fill="hsl(38 92% 50%)" /></svg>
        the anchor, fixed by definition
      </span>
    </div>
    <svg viewBox="0 0 {width} {height}" width="100%" {height} role="img"
      aria-label="Ratings with their standard errors and 95% intervals">
      <!-- Recessive grid: reference, never the subject. Every gridline drawn,
           a label only as often as the width has room for. -->
      {#each ticks as tick, i}
        <line x1={x(tick)} x2={x(tick)} y1={pad.top} y2={height - pad.bottom} stroke="hsl(217 19% 22%)" stroke-width="1" />
        {#if i % tickStep === 0}
          <text
            x={x(tick)}
            y={height - pad.bottom + 16}
            text-anchor="middle"
            class="fill-muted-foreground text-[11px] tabular-nums"
          >
            {tick.toFixed(0)}
          </text>
        {/if}
      {/each}

      {#each rated as { row, cx, cy, x1, x2, w1, w2 }, i}
        {@const figure = ratingFigure(row)}
        <!-- Each name links to its config, looking as it did until hovered. -->
        <a href="/player-configs/{row.player_config_id}">
          <text x={pad.left - 10} y={cy + 4} text-anchor="end" class="fill-foreground text-[12px] hover:underline">
            <title>{row.name}{row.is_anchor ? ' (anchor)' : ''}</title>
            {labels[i]}
          </text>
        </a>

        <!-- The 95% interval thin, the ±1 standard error thick over it. -->
        <line x1={w1} x2={w2} y1={cy} y2={cy} stroke="hsl(215 14% 60%)" stroke-width="1.5" stroke-linecap="round" />
        <line {x1} {x2} y1={cy} y2={cy} stroke="hsl(215 14% 75%)" stroke-width="4" stroke-linecap="round" />
        <!-- 2px surface ring so a dot stays legible where it overlaps its bar. -->
        <circle {cx} {cy} r="6" fill="hsl(222 22% 11%)" />
        <circle {cx} {cy} r="4.5" fill={row.is_anchor ? 'hsl(38 92% 50%)' : 'hsl(199 89% 48%)'}>
          <title>{dotTitle(row)}</title>
        </circle>

        <text x={width - 4} y={cy + 4} text-anchor="end" class="text-[12px] tabular-nums" data-testid="rating-figure">
          <tspan class="fill-foreground font-semibold">{figure.rating}</tspan>
          <tspan class="fill-muted-foreground"> {figure.error}</tspan>
        </text>
      {/each}
    </svg>
    <p class="mt-1 text-xs text-muted-foreground">
      Between any two configs, a gap of 100 points predicts about a 60–40 score, and 275 points
      75–25. Where two configs' 95% intervals do not overlap, the games tell them apart.
    </p>
  {:else}
    <p class="text-sm text-muted-foreground">No rated configs yet.</p>
  {/if}

  {#if unrated.length}
    <p class="mt-2 text-xs text-muted-foreground">
      Unrated — no chain of games connects these to the anchor, so there is no scale to place
      them on: {unrated.map((r) => r.name).join(', ')}
    </p>
  {/if}
</div>
