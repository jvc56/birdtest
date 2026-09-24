<script lang="ts">
  /**
   * Ratings as a dot plot with error bars.
   *
   * Deliberately **not** a bar chart. A bar encodes magnitude from zero, and
   * Elo has no meaningful zero — its scale is anchored wherever the pool's
   * anchor was pinned, so bar length would imply a ratio that does not exist.
   * A dot on a common scale encodes position, which is what a rating is.
   *
   * The error bar is the point of the chart as much as the dot: a config with
   * few pairs and one with millions produce identical-looking numbers, and only
   * the interval says which one to believe.
   */
  import type { RatingRow } from '$lib/api';
  import { dotPlotX, dotTitle, layoutDotPlot, PAD } from '$lib/charts/ratingDotPlot';

  export let ratings: RatingRow[] = [];

  let width = 720;

  // Unrated configs have no position on the scale, so they are listed beneath
  // the chart rather than drawn at a number that means nothing. Runaway error
  // bars are clamped (see visibleError); the table still reports the real
  // number.
  $: layout = layoutDotPlot(ratings, width);
  $: rated = layout.rated;
  $: unrated = layout.unrated;
  $: bounds = layout.bounds;
  $: ticks = layout.ticks;
  $: height = layout.height;
</script>

<div class="w-full" bind:clientWidth={width}>
  {#if rated.length}
    <svg viewBox="0 0 {width} {height}" width="100%" {height} role="img"
      aria-label="Ratings with standard-error intervals">
      <!-- Recessive grid: reference, never the subject. -->
      {#each ticks as tick}
        <line
          x1={dotPlotX(tick, bounds, width)}
          x2={dotPlotX(tick, bounds, width)}
          y1={PAD.top}
          y2={height - PAD.bottom}
          stroke="hsl(217 19% 22%)"
          stroke-width="1"
        />
        <text
          x={dotPlotX(tick, bounds, width)}
          y={height - PAD.bottom + 16}
          text-anchor="middle"
          class="fill-muted-foreground text-[11px] tabular-nums"
        >
          {tick.toFixed(0)}
        </text>
      {/each}

      {#each rated as { row, cx, cy, x1, x2 }}
        <text
          x={PAD.left - 10}
          y={cy + 4}
          text-anchor="end"
          class="fill-foreground text-[12px]"
        >
          {row.name}{row.is_anchor ? ' (anchor)' : ''}
        </text>

        <line
          {x1}
          {x2}
          y1={cy}
          y2={cy}
          stroke="hsl(215 14% 60%)"
          stroke-width="2"
          stroke-linecap="round"
        />
        <!-- 2px surface ring so a dot stays legible where it overlaps its bar. -->
        <circle cx={cx} cy={cy} r="6" fill="hsl(222 22% 11%)" />
        <circle
          cx={cx}
          cy={cy}
          r="4.5"
          fill={row.is_anchor ? 'hsl(38 92% 50%)' : 'hsl(199 89% 48%)'}
        >
          <title>{dotTitle(row)}</title>
        </circle>
      {/each}
    </svg>
    <p class="mt-1 text-xs text-muted-foreground">
      Bars are ±1 standard error. The anchor is fixed by definition, not measured.
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
