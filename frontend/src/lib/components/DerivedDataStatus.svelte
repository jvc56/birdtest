<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { api, ApiError, type JobDerivedFile } from '$lib/api';

  /**
   * The wordmaps and rack info tables a job waits on. A job that needs one is
   * not handed out until the server's copy is built, and nothing else on the
   * job page moves while it waits (no claims, so no live payloads), so this
   * reads again every few seconds until every file is built or has failed.
   */
  export let jobId: string;

  const POLL_MS = 3000;

  let files: JobDerivedFile[] | null = null;
  let error = '';
  let poll: number | undefined;
  let destroyed = false;

  async function load() {
    window.clearTimeout(poll);
    try {
      files = await api.jobDerivedData(jobId);
      error = '';
    } catch (e) {
      // The job page says a deleted job is gone; nothing more to read here.
      if (e instanceof ApiError && e.status === 404) return;
      error = e instanceof Error ? e.message : 'could not load';
    }
    if (destroyed) return;
    if (error || files?.some((f) => f.state === 'pending' || f.state === 'building')) {
      poll = window.setTimeout(load, POLL_MS);
    }
  }

  onMount(load);
  onDestroy(() => {
    destroyed = true;
    window.clearTimeout(poll);
  });

  const kind = (role: string) => (role === 'wmp' ? 'Wordmap' : 'Rack info table');
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
    <h2 class="text-lg font-medium">Wordmaps and rack info tables</h2>
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
