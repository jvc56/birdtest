<script lang="ts">
  import { workerLabel } from '$lib/format';

  export let workers: { username: string | null; anon_uuid: string | null; tasks_completed: number }[];
</script>

<table class="table">
  <thead>
    <tr><th>Contributor</th><th class="text-right">Tasks completed</th></tr>
  </thead>
  <tbody>
    {#each workers as worker}
      <tr>
        <td>{workerLabel(worker)}</td>
        <td class="text-right tabular-nums">{worker.tasks_completed.toLocaleString()}</td>
      </tr>
    {:else}
      <tr><td colspan="2" class="text-muted-foreground">No contributions yet.</td></tr>
    {/each}
  </tbody>
</table>
