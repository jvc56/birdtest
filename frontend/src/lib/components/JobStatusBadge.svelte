<script lang="ts">
  export let status: string;

  const styles: Record<string, string> = {
    active: 'bg-success/15 text-success border-success/30',
    inactive: 'bg-muted text-muted-foreground border-border',
    completed: 'bg-primary/15 text-primary border-primary/30',
    player1_better: 'bg-success/15 text-success border-success/30',
    player2_better: 'bg-success/15 text-success border-success/30',
    running: 'bg-warning/15 text-warning border-warning/30',
    paused: 'bg-muted text-muted-foreground border-border',
    undecided: 'bg-muted text-muted-foreground border-border',
    inconclusive: 'bg-muted text-muted-foreground border-border'
  };
  const labels: Record<string, string> = {
    player1_better: 'player 1 better',
    player2_better: 'player 2 better',
    undecided: 'not decided'
  };
  // Own keys only: a plain lookup of `constructor` would find a function.
  // (`Object.hasOwn` is newer than the build's browser targets.)
  const own = (map: Record<string, string>, key: string) =>
    Object.prototype.hasOwnProperty.call(map, key);
</script>

<span
  class="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium {own(
    styles,
    status
  )
    ? styles[status]
    : styles.inactive}"
>
  {own(labels, status) ? labels[status] : status.replace(/_/g, ' ')}
</span>
