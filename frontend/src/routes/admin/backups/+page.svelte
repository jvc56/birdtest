<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type BackupStatus } from '$lib/api';
  import { datetime, duration } from '$lib/format';

  let status: BackupStatus | null = null;
  let error = '';

  onMount(async () => {
    try {
      status = await api.backups();
    } catch (e) {
      error = e instanceof Error ? e.message : 'could not load backups';
    }
  });

  const mib = (bytes: number | null) =>
    bytes === null ? '—' : `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
</script>

<h1 class="mb-2 text-2xl font-semibold">Backups</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Nightly logical dumps, written to the backup bucket by a scheduled task and recorded here. This
  page is a read-only view: the backend holds no credentials for the backup bucket and cannot
  perform, alter or delete a backup. RDS point-in-time recovery runs alongside these and is not
  visible here — see <code>PLAN.md</code>’s “Backups and Restore” for which mechanism answers which failure.
</p>

{#if error}
  <p class="text-destructive">{error}</p>
{:else if !status}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div
    class="card mb-6 p-4"
    class:border-destructive={status.stale}
  >
    {#if status.last_success_at === null}
      <p class="font-medium text-destructive">No successful backup has ever been recorded.</p>
      <p class="mt-1 text-sm text-muted-foreground">
        Either the scheduled task has never run, or it has never finished. Check the
        <code>birdtest-backup</code> task's logs before this matters.
      </p>
    {:else if status.stale}
      <p class="font-medium text-destructive">
        Last successful backup was {duration(status.last_success_age_seconds)} ago.
      </p>
      <p class="mt-1 text-sm text-muted-foreground">
        A nightly schedule should never leave a gap this long. The same condition raises the
        <code>birdtest-backup-stale</code> alarm.
      </p>
    {:else}
      <p class="font-medium">
        Last successful backup {duration(status.last_success_age_seconds)} ago,
        {datetime(status.last_success_at)}.
      </p>
    {/if}
  </div>

  <div class="card overflow-x-auto p-0">
    <table class="table">
      <thead>
        <tr>
          <th>Finished</th>
          <th>Kind</th>
          <th>Location</th>
          <th class="text-right">Took</th>
          <th class="text-right">Size</th>
          <th class="text-right">Rows</th>
          <th>Result</th>
        </tr>
      </thead>
      <tbody>
        {#each status.recent as run}
          <tr>
            <td>{datetime(run.finished_at)}</td>
            <td>{run.kind}</td>
            <td class="font-mono text-xs" title={run.sha256 ?? ''}>{run.location ?? '—'}</td>
            <td class="text-right tabular-nums">{duration(run.duration_seconds)}</td>
            <td class="text-right tabular-nums">{mib(run.dump_bytes)}</td>
            <td class="text-right tabular-nums">{run.total_rows.toLocaleString()}</td>
            <td class={run.ok ? '' : 'text-destructive'}>{run.ok ? 'ok' : 'failed'}</td>
          </tr>
        {:else}
          <tr><td colspan="7" class="text-muted-foreground">No backup runs recorded.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}
