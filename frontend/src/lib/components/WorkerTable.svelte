<script lang="ts">
  import { workerLabel } from '$lib/format';

  export let workers: { username: string | null; anon_id: string | null; tasks_completed: number }[];
</script>

<!-- Scrolls in its own box on a phone rather than widening the page, and a
     long username breaks, so the column the list is ranked by stays in view. -->
<div class="overflow-x-auto">
<table class="table">
  <thead>
    <tr><th>Contributor</th><th class="text-right">Tasks completed</th></tr>
  </thead>
  <tbody>
    {#each workers as worker}
      <tr>
        <td class="break-all">{workerLabel(worker)}</td>
        <td class="text-right tabular-nums">{worker.tasks_completed.toLocaleString()}</td>
      </tr>
    {:else}
      <tr><td colspan="2" class="text-muted-foreground">No contributions yet.</td></tr>
    {/each}
  </tbody>
</table>
</div>
