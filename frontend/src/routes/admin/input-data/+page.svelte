<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api, errorText, type ImportDetail, type InputData } from '$lib/api';
  import { refreshSession } from '$lib/auth';
  import { createImportWatcher } from '$lib/importWatch';

  let files: InputData[] = [];
  let error = '';

  // The import wizard: date -> progress poll -> staged diff -> confirm.
  let tarballDate = '';
  let gitRef = 'main';
  let current: ImportDetail | null = null;
  let busy = false;
  // Rows the last confirm inserted, as the server counted them.
  let inserted: number | null = null;

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
  // Forgets the stored import only if it is still `id`: another started since
  // is kept.
  const forgetIfStored = (id: string) => {
    try {
      if (localStorage.getItem(IMPORT_KEY) === id) localStorage.removeItem(IMPORT_KEY);
    } catch {
      // Storage unavailable: nothing stored.
    }
  };
  const remember = (id: string | null) => {
    try {
      if (id) localStorage.setItem(IMPORT_KEY, id);
      else localStorage.removeItem(IMPORT_KEY);
    } catch {
      // Storage unavailable (a private window): nothing to resume, no harm.
    }
  };
  // Polls the import until it stops running (lib/importWatch.ts).
  const watcher = createImportWatcher({
    read: (id) => api.getImport(id),
    onState: (detail) => {
      current = detail;
      error = '';
    },
    onError: (e) => (error = errorText(e)),
    forget: () => remember(null),
    // Refreshed, the session store lets the admin layout send the admin to
    // sign in and back here, where the kept id resumes.
    signedOut: () => refreshSession()
  });

  // The import in this browser's storage, if any, is watched again: its first
  // read says what it is now.
  function resume() {
    let id: string | null = null;
    try {
      id = localStorage.getItem(IMPORT_KEY);
    } catch {
      return;
    }
    if (id) watcher.watch(id);
  }
  onMount(() => {
    load();
    resume();
  });
  onDestroy(() => watcher.stop());

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
      inserted = null;
      // The previous import off the page at once: left there, its Insert
      // stayed live until the new one's first read, and confirming it forgot
      // the new one.
      current = null;
      watcher.watch(started.id);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function confirm() {
    if (!current) return;
    // This import, throughout: `current` may be another one by the time an
    // await returns.
    const id = current.id;
    busy = true;
    error = '';
    try {
      const confirmed = await api.confirmImport(id);
      forgetIfStored(id);
      // What the server inserted, not what was staged: rows another import
      // confirmed first are skipped.
      inserted = confirmed.inserted;
      if (current?.id === id) {
        // Confirmed whatever the next read says: a failed read left the
        // button live, and a second click was a 409.
        current = { ...current, state: 'confirmed' };
        const read = await api.getImport(id);
        if (current?.id === id) current = read;
      }
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
      <p class="text-sm">
        Confirmed.{inserted !== null ? ` ${inserted} rows inserted.` : ''}
      </p>
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
