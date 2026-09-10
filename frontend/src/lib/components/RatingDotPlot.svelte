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

  export let ratings: RatingRow[] = [];

  const ROW_HEIGHT = 28;
  const PAD = { top: 8, right: 24, bottom: 28, left: 180 };

  // Unrated configs have no position on the scale, so they are listed beneath
  // the chart rather than drawn at a number that means nothing.
  $: rated = ratings.filter((r) => r.connected_to_anchor);
  $: unrated = ratings.filter((r) => !r.connected_to_anchor);

  $: height = PAD.top + PAD.bottom + rated.length * ROW_HEIGHT;
  $: bounds = (() => {
    if (!rated.length) return { lo: 0, hi: 1 };
    const los = rated.map((r) => r.rating - visibleError(r));
    const his = rated.map((r) => r.rating + visibleError(r));
    const lo = Math.min(...los);
    const hi = Math.max(...his);
    const pad = Math.max(10, (hi - lo) * 0.08);
    return { lo: lo - pad, hi: hi + pad };
  })();

  /** Clamp runaway intervals so one barely-measured config cannot flatten the
   *  scale for everyone else; the table still reports the real number. */
  function visibleError(row: RatingRow): number {
    return Math.min(row.stderr, 400);
  }

  const x = (value: number, lo: number, hi: number, width: number) =>
    PAD.left + ((value - lo) / (hi - lo)) * (width - PAD.left - PAD.right);

  let width = 720;

  $: ticks = (() => {
    const span = bounds.hi - bounds.lo;
    const step = Math.pow(10, Math.floor(Math.log10(span / 4)));
    const nice = [1, 2, 5, 10].map((m) => m * step).find((s) => span / s <= 6) ?? step;
    const out: number[] = [];
    for (let t = Math.ceil(bounds.lo / nice) * nice; t <= bounds.hi; t += nice) out.push(t);
    return out;
  })();
</script>

<div class="w-full" bind:clientWidth={width}>
  {#if rated.length}
    <svg viewBox="0 0 {width} {height}" width="100%" {height} role="img"
      aria-label="Ratings with standard-error intervals">
      <!-- Recessive grid: reference, never the subject. -->
      {#each ticks as tick}
        <line
          x1={x(tick, bounds.lo, bounds.hi, width)}
          x2={x(tick, bounds.lo, bounds.hi, width)}
          y1={PAD.top}
          y2={height - PAD.bottom}
          stroke="hsl(217 19% 22%)"
          stroke-width="1"
        />
        <text
          x={x(tick, bounds.lo, bounds.hi, width)}
          y={height - PAD.bottom + 16}
          text-anchor="middle"
          class="fill-muted-foreground text-[11px] tabular-nums"
        >
          {tick.toFixed(0)}
        </text>
      {/each}

      {#each rated as row, i}
        {@const cy = PAD.top + i * ROW_HEIGHT + ROW_HEIGHT / 2}
        {@const cx = x(row.rating, bounds.lo, bounds.hi, width)}
        {@const err = visibleError(row)}
        <text
          x={PAD.left - 10}
          y={cy + 4}
          text-anchor="end"
          class="fill-foreground text-[12px]"
        >
          {row.name}{row.is_anchor ? ' (anchor)' : ''}
        </text>

        <line
          x1={x(row.rating - err, bounds.lo, bounds.hi, width)}
          x2={x(row.rating + err, bounds.lo, bounds.hi, width)}
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
          <title>
            {row.name}: {row.rating.toFixed(1)} ± {row.stderr.toFixed(1)} over {row.pairs_played.toLocaleString()} pairs
          </title>
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
