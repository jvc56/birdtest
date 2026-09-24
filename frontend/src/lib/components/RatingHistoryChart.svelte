<script lang="ts">
  /**
   * Rating over time, one line per player config.
   *
   * The time axis exists because runs are snapshotted rather than mutated in
   * place — there is a row per config per fit, so history is free.
   *
   * At most SERIES_CAP configs are drawn. A categorical palette is a fixed
   * list, not a generator: past its length the honest move is to show fewer
   * series and say so, rather than invent hues nobody can tell apart.
   */
  import type { RatingHistoryPoint } from '$lib/api';
  import {
    groupHistory,
    HEIGHT,
    historyDomain,
    PAD,
    seriesPath,
    historyX as sx,
    historyY as sy
  } from '$lib/charts/ratingHistory';

  export let history: RatingHistoryPoint[] = [];

  let width = 720;

  // Ranked by latest rating so the cap keeps the configs a reader is looking
  // for; colour follows the config's identity rather than its rank in this
  // view (see assignColors).
  $: grouped = groupHistory(history);
  $: domain = historyDomain(grouped.series);
</script>

<div class="w-full" bind:clientWidth={width}>
  {#if domain}
    <svg viewBox="0 0 {width} {HEIGHT}" width="100%" height={HEIGHT} role="img"
      aria-label="Rating over time by player config">
      {#each [domain.r0, (domain.r0 + domain.r1) / 2, domain.r1] as tick}
        <line
          x1={PAD.left}
          x2={width - PAD.right}
          y1={sy(tick, domain)}
          y2={sy(tick, domain)}
          stroke="hsl(217 19% 22%)"
        />
        <text
          x={PAD.left - 8}
          y={sy(tick, domain) + 4}
          text-anchor="end"
          class="fill-muted-foreground text-[11px] tabular-nums">{tick.toFixed(0)}</text
        >
      {/each}

      {#each grouped.series as series}
        <path
          d={seriesPath(series, domain, width)}
          fill="none"
          stroke={series.color}
          stroke-width="2"
          stroke-linejoin="round"
        />
        <!-- Direct labels: identity is never carried by colour alone. -->
        {#if series.points.length}
          {@const last = series.points[series.points.length - 1]}
          <circle
            cx={sx(last.t, domain, width)}
            cy={sy(last.rating, domain)}
            r="3.5"
            fill={series.color}
          />
          <text
            x={sx(last.t, domain, width) + 8}
            y={sy(last.rating, domain) + 4}
            class="fill-foreground text-[11px]">{series.name}</text
          >
        {/if}
      {/each}
    </svg>
    {#if grouped.hidden}
      <p class="mt-1 text-xs text-muted-foreground">
        {grouped.hidden} more config{grouped.hidden === 1 ? '' : 's'} not shown — the table below
        lists every one.
      </p>
    {/if}
  {:else}
    <p class="text-sm text-muted-foreground">
      Needs at least two fits before there is a history to draw.
    </p>
  {/if}
</div>
