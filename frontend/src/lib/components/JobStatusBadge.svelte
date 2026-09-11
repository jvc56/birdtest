<script lang="ts">
  export let status: string;

  const styles: Record<string, string> = {
    active: 'bg-success/15 text-success border-success/30',
    inactive: 'bg-muted text-muted-foreground border-border',
    completed: 'bg-primary/15 text-primary border-primary/30',
    passed: 'bg-success/15 text-success border-success/30',
    failed: 'bg-destructive/15 text-destructive border-destructive/30',
    running: 'bg-warning/15 text-warning border-warning/30',
    terminated_at_max: 'bg-muted text-muted-foreground border-border'
  };
</script>

<span
  class="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium {styles[
    status
  ] ?? styles.inactive}"
>
  {status.replace(/_/g, ' ')}
</span>
