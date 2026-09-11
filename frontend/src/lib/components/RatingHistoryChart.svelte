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

  export let history: RatingHistoryPoint[] = [];

  const SERIES_CAP = 6;
  // Validated for this surface (dark, #151922) at all-pairs CVD separation.
  // See scripts/validate_palette.js in the dataviz reference.
  const PALETTE = ['#027FD1', '#BF2001', '#7902F0', '#03856D', '#E103AE', '#A09100'];

  const PAD = { top: 12, right: 132, bottom: 28, left: 52 };
  const HEIGHT = 260;
  let width = 720;

  interface Series {
    id: string;
    name: string;
    color: string;
    points: { t: number; rating: number }[];
  }

  $: grouped = (() => {
    const byConfig = new Map<string, RatingHistoryPoint[]>();
    for (const point of history) {
      const list = byConfig.get(point.player_config_id) ?? [];
      list.push(point);
      byConfig.set(point.player_config_id, list);
    }
    // Rank by latest rating so the cap keeps the configs a reader is looking
    // for, and colour follows the config rather than its rank in this view.
    const ranked = [...byConfig.entries()].sort(
      (a, b) => (b[1].at(-1)?.rating ?? 0) - (a[1].at(-1)?.rating ?? 0)
    );
    return {
      series: ranked.slice(0, SERIES_CAP).map(([id, points], i): Series => ({
        id,
        name: points[0].name,
        color: PALETTE[i],
        points: points.map((p) => ({ t: Date.parse(p.computed_at), rating: p.rating }))
      })),
      hidden: Math.max(0, ranked.length - SERIES_CAP)
    };
  })();

  $: domain = (() => {
    const all = grouped.series.flatMap((s) => s.points);
    if (all.length < 2) return null;
    const ts = all.map((p) => p.t);
    const rs = all.map((p) => p.rating);
    const lo = Math.min(...rs);
    const hi = Math.max(...rs);
    const pad = Math.max(5, (hi - lo) * 0.1);
    return { t0: Math.min(...ts), t1: Math.max(...ts), r0: lo - pad, r1: hi + pad };
  })();

  const sx = (t: number, d: NonNullable<typeof domain>, w: number) =>
    PAD.left + (d.t1 === d.t0 ? 0 : ((t - d.t0) / (d.t1 - d.t0)) * (w - PAD.left - PAD.right));
  const sy = (r: number, d: NonNullable<typeof domain>) =>
    PAD.top + (1 - (r - d.r0) / (d.r1 - d.r0)) * (HEIGHT - PAD.top - PAD.bottom);
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
          d={series.points
            .map((p, i) => `${i === 0 ? 'M' : 'L'}${sx(p.t, domain, width)},${sy(p.rating, domain)}`)
            .join(' ')}
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
