<script lang="ts">
  import { LayerCake, Svg } from 'layercake';
  import { scaleBand } from 'd3-scale';
  import Bars from './Bars.svelte';
  import AxisY from './AxisY.svelte';

  export let wins: number;
  export let losses: number;
  export let draws: number;

  const colors: Record<string, string> = {
    Wins: 'hsl(142 71% 45%)',
    Losses: 'hsl(0 72% 51%)',
    Draws: 'hsl(215 14% 45%)'
  };

  $: data = [
    { label: 'Wins', value: wins },
    { label: 'Losses', value: losses },
    { label: 'Draws', value: draws }
  ];
</script>

<div class="h-32 w-full">
  <LayerCake
    {data}
    x="value"
    y="label"
    xDomain={[0, null]}
    yScale={scaleBand().paddingInner(0.25)}
    padding={{ top: 4, right: 12, bottom: 4, left: 60 }}
  >
    <Svg>
      <AxisY />
      <Bars fill={(d) => colors[d.label]} />
    </Svg>
  </LayerCake>
</div>
