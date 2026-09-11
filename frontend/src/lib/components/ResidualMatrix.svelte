<script lang="ts">
  /**
   * Actual minus predicted score, per head-to-head.
   *
   * This is the honesty panel. A single number per player cannot represent a
   * non-transitive result — A beats B, B beats C, C beats A — and no scalar
   * rating system can. The fit returns the best scalar approximation anyway,
   * so the place the model is lying has to be visible somewhere, and this is
   * it: a rock-paper-scissors triangle appears as a set of large, sign-flipped
   * cells that no assignment of ratings could remove.
   *
   * Diverging encoding, because the quantity has a meaningful midpoint (zero:
   * the model was right) and two directions: two hues with a neutral grey at
   * the middle, never a rainbow.
   */
  import type { RatingResidual, RatingRow } from '$lib/api';

  export let residuals: RatingResidual[] = [];
  export let ratings: RatingRow[] = [];

  /** Anything past this is a disagreement worth a reader's attention. */
  const NOTABLE = 0.05;
  const SCALE_MAX = 0.25;

  $: names = new Map(ratings.map((r) => [r.player_config_id, r.name]));

  /** Warm for "scored above prediction", cool for below, grey at zero. */
  function fill(delta: number): string {
    const t = Math.min(Math.abs(delta) / SCALE_MAX, 1);
    if (t < 0.04) return 'hsl(217 19% 20%)';
    const [h, s] = delta > 0 ? [25, 85] : [205, 85];
    return `hsl(${h} ${s}% ${20 + t * 32}%)`;
  }

  $: notable = residuals.filter((r) => Math.abs(r.actual - r.predicted) >= NOTABLE);
</script>

{#if residuals.length}
  <div class="space-y-3">
    <div class="overflow-x-auto">
      <table class="table text-xs">
        <thead>
          <tr>
            <th>Head-to-head</th>
            <th class="text-right">Pairs</th>
            <th class="text-right">Actual</th>
            <th class="text-right">Predicted</th>
            <th class="text-right">Residual</th>
            <th class="w-32"></th>
          </tr>
        </thead>
        <tbody>
          {#each residuals as cell}
            {@const delta = cell.actual - cell.predicted}
            <tr>
              <td>{names.get(cell.row) ?? '?'} vs {names.get(cell.col) ?? '?'}</td>
              <td class="text-right tabular-nums">{cell.pairs.toLocaleString()}</td>
              <td class="text-right tabular-nums">{(100 * cell.actual).toFixed(1)}%</td>
              <td class="text-right tabular-nums">{(100 * cell.predicted).toFixed(1)}%</td>
              <td
                class="text-right tabular-nums"
                class:text-warning={Math.abs(delta) >= NOTABLE}
              >
                {delta > 0 ? '+' : ''}{(100 * delta).toFixed(1)}
              </td>
              <td>
                <!-- The bar restates the number; the number is not colour-alone. -->
                <div class="flex h-3 items-center">
                  <div class="relative h-2 w-full rounded-sm bg-muted">
                    <div
                      class="absolute top-0 h-2 rounded-sm"
                      style="background:{fill(delta)};
                             left:{delta > 0 ? 50 : 50 - Math.min(Math.abs(delta) / SCALE_MAX, 1) * 50}%;
                             width:{Math.min(Math.abs(delta) / SCALE_MAX, 1) * 50}%"
                    ></div>
                  </div>
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    {#if notable.length >= 3}
      <p class="text-xs text-warning">
        {notable.length} head-to-heads are more than {(100 * NOTABLE).toFixed(0)} points from what
        the ratings predict. That is the signature of a non-transitive pool — configs that beat
        some opponents and lose to others in a way no single number per player can express. Read
        the ratings as a summary here, not as a ranking.
      </p>
    {/if}
  </div>
{:else}
  <p class="text-sm text-muted-foreground">No head-to-head results in this pool yet.</p>
{/if}
