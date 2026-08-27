<script lang="ts">
  import { getContext } from 'svelte';

  // LayerCake puts its scales and sized data on context; this component is the
  // "mark" layer that turns them into rectangles.
  const { data, xGet, yGet, xScale, yScale } = getContext<any>('LayerCake');

  export let fill: (d: any) => string = () => 'hsl(199 89% 48%)';
</script>

<g class="bars">
  {#each $data as row}
    <rect
      x={$xScale(0)}
      y={$yGet(row)}
      width={Math.max(0, $xGet(row) - $xScale(0))}
      height={$yScale.bandwidth()}
      fill={fill(row)}
      rx="2"
    />
  {/each}
</g>
