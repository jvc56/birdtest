<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { api, ApiError, type JobDerivedFile } from '$lib/api';
  import { BUSY_POLL_MS, IDLE_POLL_MS, createPoller } from '$lib/poller';
  import { derivedKind as kind } from '$lib/format';

  /**
   * The wordmaps, rack info tables and word info tables a job waits on. A job that needs one is
   * not handed out until the server's copy is built, and nothing else on the
   * job page moves while it waits (no claims, so no live payloads), so this
   * reads again every few seconds while a file is queued or building, and
   * less often otherwise: a failed build retried, or a file queued when the
   * job is activated, shows up by itself.
   */
  export let jobId: string;

  let files: JobDerivedFile[] | null = null;
  let error = '';

  const gone = (e: unknown) => e instanceof ApiError && e.status === 404;
  const poller = createPoller(
    {
      read: () => api.jobDerivedData(jobId),
      onValue: (read) => {
        files = read;
        error = '';
      },
      onError: (e) => {
        // The job page says a deleted job is gone; nothing more to read here.
        if (!gone(e)) error = e instanceof Error ? e.message : 'could not load';
      },
      delay: (read, failure) => {
        if (failure) return gone(failure.error) ? null : BUSY_POLL_MS;
        return read?.some((f) => f.state === 'pending' || f.state === 'building')
          ? BUSY_POLL_MS
          : IDLE_POLL_MS;
      }
    },
    document
  );

  onMount(() => poller.start());
  onDestroy(() => poller.stop());

  const label: Record<string, string> = {
    pending: 'queued',
    building: 'building…',
    built: 'built',
    failed: 'failed'
  };

  $: waiting = (files ?? []).filter((f) => f.state === 'pending' || f.state === 'building');
  $: failed = (files ?? []).filter((f) => f.state === 'failed');
</script>

{#if error}
  <p class="text-sm text-destructive">Could not check this job's derived files: {error}</p>
{:else if files && files.length}
  <div
    class="card space-y-2"
    class:border-destructive={failed.length > 0}
    role="status"
    aria-live="polite"
  >
    <h2 class="text-lg font-medium">Derived files</h2>
    {#if failed.length}
      <p class="text-sm text-destructive">
        A build has given up, so this job hands out no work. Retry it on
        <a href="/admin/derived-data">Derived data</a>.
      </p>
    {:else if waiting.length}
      <p class="text-sm">
        The server is building {waiting.length === 1 ? 'a file' : 'files'} this job needs; no work
        is handed out until {waiting.length === 1 ? 'it is' : 'they are'} built. A rack info table
        takes one to three minutes.
      </p>
    {:else}
      <p class="text-sm text-muted-foreground">
        Built. Each contributor also builds its own copy the first time it gets one of this job's
        tasks, which takes it the same minutes.
      </p>
    {/if}
    <ul class="space-y-1 text-sm">
      {#each files as file (file.role + ' ' + file.name)}
        <li class:text-destructive={file.state === 'failed'}>
          {kind(file.role)} <code>{file.name}</code>: {label[file.state] ?? file.state}
          {#if file.state === 'failed' && file.error}
            <span class="text-xs">({file.error})</span>
          {/if}
        </li>
      {/each}
    </ul>
  </div>
{/if}
