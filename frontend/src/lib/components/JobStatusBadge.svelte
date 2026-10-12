<script lang="ts">
  export let status: string;

  const styles: Record<string, string> = {
    active: 'bg-success/15 text-success border-success/30',
    inactive: 'bg-muted text-muted-foreground border-border',
    completed: 'bg-primary/15 text-primary border-primary/30'
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
  {status.replace(/_/g, ' ')}
</span>
