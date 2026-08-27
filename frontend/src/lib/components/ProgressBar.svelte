<script lang="ts">
  export let value: number;
  export let max: number;
  export let label: string = '';

  $: pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
</script>

<div>
  {#if label}
    <div class="mb-1 flex justify-between text-xs text-muted-foreground">
      <span>{label}</span>
      <span class="tabular-nums">{value.toLocaleString()} / {max.toLocaleString()}</span>
    </div>
  {/if}
  <div class="h-2 w-full overflow-hidden rounded-full bg-muted">
    <div class="h-full rounded-full bg-primary transition-[width]" style="width: {pct}%"></div>
  </div>
</div>
