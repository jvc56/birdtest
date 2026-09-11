<script lang="ts">
  import { getContext } from 'svelte';

  const { yScale, xScale } = getContext<any>('LayerCake');
</script>

<g>
  {#each $yScale.domain() as tick}
    <text
      x={$xScale(0) - 8}
      y={$yScale(tick) + $yScale.bandwidth() / 2}
      text-anchor="end"
      dominant-baseline="middle"
      class="fill-current text-[11px] text-muted-foreground">{tick}</text
    >
  {/each}
</g>
