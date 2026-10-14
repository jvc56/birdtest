<script lang="ts">
  /**
   * A job's contributors: each one's movegens, compute time and tasks on the
   * job, and with `totals`, a Total row of the job's own figures -- every
   * contributor's, so it is right when the list stops short of them all.
   */
  import { computeTime, exactCount, workerLabel } from '$lib/format';

  export let workers: {
    username: string | null;
    anon_id: string | null;
    tasks_completed: number;
    compute_seconds: number;
    movegens: number;
  }[];
  export let totals: { movegens: number; compute_seconds: number; tasks_completed: number } | null = null;
</script>

<!-- Scrolls in its own box on a phone rather than widening the page, and a
     long username breaks, so the column the list is ranked by stays in view. -->
<div class="overflow-x-auto">
<table class="table">
  <thead>
    <tr>
      <th>Contributor</th>
      <th class="hidden text-right sm:table-cell">Movegens</th>
      <th class="hidden text-right sm:table-cell">Compute time</th>
      <th class="text-right">Tasks completed</th>
    </tr>
  </thead>
  <tbody>
    {#each workers as worker}
      <tr>
        <td class="break-all">{workerLabel(worker)}</td>
        <td class="hidden whitespace-nowrap text-right tabular-nums sm:table-cell">{exactCount(worker.movegens)}</td>
        <td class="hidden whitespace-nowrap text-right tabular-nums sm:table-cell">
          {computeTime(worker.compute_seconds)}
        </td>
        <td class="text-right tabular-nums">{worker.tasks_completed.toLocaleString()}</td>
      </tr>
    {:else}
      <tr><td colspan="4" class="text-muted-foreground">No contributions yet.</td></tr>
    {/each}
  </tbody>
  {#if totals && workers.length}
    <tfoot>
      <tr class="font-semibold" data-testid="worker-totals">
        <td>Total</td>
        <td class="hidden whitespace-nowrap text-right tabular-nums sm:table-cell">{exactCount(totals.movegens)}</td>
        <td class="hidden whitespace-nowrap text-right tabular-nums sm:table-cell">
          {computeTime(totals.compute_seconds)}
        </td>
        <td class="text-right tabular-nums">{totals.tasks_completed.toLocaleString()}</td>
      </tr>
    </tfoot>
  {/if}
</table>
</div>
