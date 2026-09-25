<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api, ApiError, errorText, type ImportDetail, type InputData } from '$lib/api';

  let files: InputData[] = [];
  let error = '';

  // The import wizard: date -> progress poll -> staged diff -> confirm.
  let tarballDate = '';
  let gitRef = 'main';
  let current: ImportDetail | null = null;
  let busy = false;
  let poll: ReturnType<typeof setInterval> | null = null;

  $: newRows = current?.files.filter((f) => f.disposition === 'new') ?? [];
  $: collisions = current?.files.filter((f) => f.disposition === 'collision') ?? [];
  $: knownRows = current?.files.filter((f) => f.disposition === 'known') ?? [];

  async function load() {
    try {
      files = await api.inputData();
    } catch (e) {
      error = `Could not load the input data: ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  // The import in progress, remembered in this browser: there is no list of
  // imports, so a reload (or a poll that gave up) otherwise lost a staged
  // import, and the only way on was to download it again.
  const IMPORT_KEY = 'birdtest:input-data-import';
  const remember = (id: string | null) => {
    try {
      if (id) localStorage.setItem(IMPORT_KEY, id);
      else localStorage.removeItem(IMPORT_KEY);
    } catch {
      // Storage unavailable (a private window): nothing to resume, no harm.
    }
  };
  // A refusal that asking again will not change: the import is gone, or the id
  // is not one. Anything else -- a deploy's 503, a network blip -- is worth
  // asking again about, and must not forget a staged import.
  const isFinal = (e: unknown) => {
    const status = e instanceof ApiError ? e.status : 0;
    return status >= 400 && status < 500 && status !== 408 && status !== 429;
  };
  async function resume() {
    let id: string | null = null;
    try {
      id = localStorage.getItem(IMPORT_KEY);
    } catch {
      return;
    }
    if (!id) return;
    try {
      current = await api.getImport(id);
      if (current.state === 'running') watch(id);
      else if (current.state !== 'staged') remember(null);
    } catch (e) {
      if (isFinal(e)) remember(null);
      else {
        error = errorText(e);
        watch(id);
      }
    }
  }
  onMount(() => {
    load();
    resume();
  });
  onDestroy(() => poll && clearInterval(poll));

  function watch(id: string) {
    poll && clearInterval(poll);
    poll = setInterval(async () => {
      try {
        current = await api.getImport(id);
        error = '';
        if (current.state !== 'running') {
          poll && clearInterval(poll);
          poll = null;
          if (current.state !== 'staged') remember(null);
        }
      } catch (e) {
        error = errorText(e);
        // Only a refusal that asking again will not change ends the watch.
        // A deploy's 503 or a network blip used to end it for good, and with
        // no list of imports the staged one could not be found again: the
        // admin downloaded the ~94 MB again instead.
        if (isFinal(e)) {
          poll && clearInterval(poll);
          poll = null;
          remember(null);
        }
      }
    }, 1000);
  }

  async function start() {
    busy = true;
    error = '';
    try {
      // Returns as soon as the ref resolves; the ~94 MB download runs in the
      // background and this page polls for it.
      // An empty ref is the server's default ("main") rather than an error.
      const started = await api.startImport({
        tarball_date: tarballDate.trim(),
        git_ref: gitRef.trim() || undefined
      });
      remember(started.id);
      // Watched before the first read: if that read fails, the poll still
      // finds the import.
      watch(started.id);
      current = await api.getImport(started.id);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function confirm() {
    if (!current) return;
    busy = true;
    error = '';
    try {
      await api.confirmImport(current.id);
      remember(null);
      // Confirmed whatever the next read says: a failed read left the button
      // live, and a second click was a 409.
      current = { ...current, state: 'confirmed' };
      current = await api.getImport(current.id);
      await load();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function remove(file: InputData) {
    error = '';
    if (!confirm2(`Delete ${file.path}?`)) return;
    try {
      await api.deleteInputData(file.id);
      await load();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  // `confirm` is taken by the import action above.
  const confirm2 = (message: string) => window.confirm(message);

  const mib = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
</script>

<h1 class="mb-2 text-2xl font-semibold">Input data</h1>
<p class="mb-6 text-sm text-muted-foreground">
  One row per distinct file, identified by content. Jobs and player configs pin these rows, so a
  worker whose copy of a file does not match the digest declines the task instead of contributing
  something incomparable.
</p>

{#if error}<p class="mb-4 text-destructive">{error}</p>{/if}

<div class="card mb-6 space-y-3">
  <h2 class="font-semibold">Import a tarball</h2>
  <div class="grid grid-cols-3 gap-3">
    <div>
      <label class="label" for="date">Version (YYYYMMDD)</label>
      <input id="date" class="input" bind:value={tarballDate} placeholder="20260101" />
    </div>
    <div>
      <label class="label" for="ref">Ref</label>
      <input id="ref" class="input" bind:value={gitRef} placeholder="main" />
      <p class="mt-1 text-xs text-muted-foreground">
        Resolved to a commit at import time, so the record names a commit and never a branch.
      </p>
    </div>
    <div class="flex items-end">
      <button class="btn-primary" disabled={busy || !tarballDate} on:click={start}>
        {busy ? 'Working…' : 'Fetch and diff'}
      </button>
    </div>
  </div>

  {#if current}
    <p class="text-xs text-muted-foreground">Import <span class="font-mono">{current.id}</span></p>
    {#if current.state === 'running'}
      <p class="text-sm">
        Downloading… {mib(current.progress_bytes)}, {current.progress_entries} files hashed.
      </p>
    {:else if current.state === 'cancelled'}
      <p class="text-sm text-muted-foreground">
        This import was cancelled before it was confirmed{current.error ? `: ${current.error}` : '.'}
      </p>
    {:else if current.state === 'failed'}
      <p class="text-destructive">Import failed: {current.error}</p>
    {:else if current.state === 'confirmed'}
      <p class="text-sm">Confirmed. {newRows.length + collisions.length} rows inserted.</p>
    {:else if current.state === 'staged'}
      <div class="space-y-2 text-sm">
        <p>
          <strong>{newRows.length}</strong> new,
          <strong>{collisions.length}</strong> changed,
          <strong>{knownRows.length}</strong> already known.
        </p>
        {#if collisions.length}
          <p class="text-muted-foreground">
            A changed file is a path already known under different bytes — either a legitimate data
            update, or a tarball re-cut under a name that was already used. Worth a second look
            before confirming.
          </p>
        {/if}
        <div class="max-h-64 overflow-y-auto rounded border">
          <table class="table">
            <thead>
              <tr><th>Path</th><th>Role</th><th>Digest</th><th class="text-right">Bytes</th><th></th></tr>
            </thead>
            <tbody>
              {#each [...collisions, ...newRows] as file}
                <tr>
                  <td>{file.path}</td>
                  <td>{file.role}</td>
                  <td class="font-mono text-xs">{file.sha256.slice(0, 12)}</td>
                  <td class="text-right tabular-nums">{file.bytes.toLocaleString()}</td>
                  <td>{file.disposition}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <button class="btn-primary" disabled={busy} on:click={confirm}>
          Insert {newRows.length + collisions.length} rows
        </button>
      </div>
    {/if}
  {/if}
</div>

<div class="card overflow-x-auto p-0">
  <table class="table">
    <thead>
      <tr>
        <th>Path</th><th>Role</th><th>Name</th><th>From</th><th>Digest</th>
        <th class="text-right">Bytes</th><th class="text-right">Pinned by</th><th></th>
      </tr>
    </thead>
    <tbody>
      {#each files as file}
        <tr>
          <td>{file.path}</td>
          <td>{file.role}</td>
          <td>{file.name}</td>
          <td>data-{file.tarball_date} or later</td>
          <td class="font-mono text-xs">{file.sha256.slice(0, 12)}</td>
          <td class="text-right tabular-nums">{file.bytes.toLocaleString()}</td>
          <td class="text-right tabular-nums">{file.references}</td>
          <td class="text-right">
            <button class="btn-destructive" disabled={file.references > 0} on:click={() => remove(file)}>
              Delete
            </button>
          </td>
        </tr>
      {:else}
        <tr><td colspan="8" class="text-muted-foreground">Nothing imported yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>
