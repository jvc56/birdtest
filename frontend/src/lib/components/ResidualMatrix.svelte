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
  import {
    formatResidual,
    isNotable,
    NOTABLE,
    isNonTransitive,
    significantResiduals,
    residual,
    residualBar,
    residualFill,
    sortResiduals
  } from '$lib/charts/residuals';

  export let residuals: RatingResidual[] = [];
  export let ratings: RatingRow[] = [];

  $: names = new Map(ratings.map((r) => [r.player_config_id, r.name]));
  $: sorted = sortResiduals(residuals);
  $: significant = significantResiduals(residuals);
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
          {#each sorted as cell}
            {@const delta = residual(cell)}
            {@const bar = residualBar(delta)}
            <tr>
              <td>{names.get(cell.row) ?? '?'} vs {names.get(cell.col) ?? '?'}</td>
              <td class="text-right tabular-nums">{cell.pairs.toLocaleString()}</td>
              <td class="text-right tabular-nums">{(100 * cell.actual).toFixed(1)}%</td>
              <td class="text-right tabular-nums">{(100 * cell.predicted).toFixed(1)}%</td>
              <td class="text-right tabular-nums" class:text-warning={isNotable(delta)}>
                {formatResidual(delta)}
              </td>
              <td>
                <!-- The bar restates the number; the number is not colour-alone. -->
                <div class="flex h-3 items-center">
                  <div class="relative h-2 w-full rounded-sm bg-muted">
                    <div
                      class="absolute top-0 h-2 rounded-sm"
                      style="background:{residualFill(delta)};
                             left:{bar.left}%;
                             width:{bar.width}%"
                    ></div>
                  </div>
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    {#if isNonTransitive(residuals)}
      <p class="text-xs text-warning">
        {significant.length} head-to-heads are more than {(100 * NOTABLE).toFixed(0)} points from
        what the ratings predict, on enough pairs that it is not chance. That is the signature of a non-transitive pool — configs that beat
        some opponents and lose to others in a way no single number per player can express. Read
        the ratings as a summary here, not as a ranking.
      </p>
    {/if}
  </div>
{:else}
  <p class="text-sm text-muted-foreground">No head-to-head results in this pool yet.</p>
{/if}
