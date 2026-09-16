<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type DerivedData } from '$lib/api';
  import { datetime } from '$lib/format';

  let rows: DerivedData[] = [];
  let error = '';
  let busy = '';

  async function load() {
    try {
      rows = await api.derivedData();
    } catch (e) {
      error = e instanceof Error ? e.message : 'could not load derived data';
    }
  }

  onMount(load);

  async function retry(row: DerivedData) {
    busy = `${row.role} ${row.name}`;
    try {
      await api.retryDerivedData(row.role, row.name);
      await load();
    } catch (e) {
      error = e instanceof Error ? e.message : 'could not retry that build';
    } finally {
      busy = '';
    }
  }

  const mib = (bytes: number | null) =>
    bytes === null ? '—' : `${(bytes / 1024 / 1024).toFixed(0)} MiB`;

  const kind = (role: string) => (role === 'wmp' ? 'Wordmap' : 'Rack info table');

  $: waiting = rows.filter((r) => r.state === 'pending' || r.state === 'building');
  $: failed = rows.filter((r) => r.state === 'failed');
</script>

<h1 class="mb-2 text-2xl font-semibold">Derived data</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Wordmaps and rack info tables are built on each contributor’s own machine and are far too
  large to ship — 179&nbsp;MB and 1.9&nbsp;GB for CSW24 — so the server builds a reference copy
  with its pinned MAGPIE and publishes the SHA-256 for workers to reproduce. A worker whose
  own build has different bytes declines the task with <code>derived_mismatch</code> rather
  than playing with a file nobody checked.
</p>
<p class="mb-6 text-sm text-muted-foreground">
  <strong>A job that needs one of these is not dispatched until it says “built”.</strong>
  That is the usual reason an active job is handing out no work. Builds run in a scheduled
  task (<code>birdtest-derived-builder</code>); a wordmap takes a couple of seconds and a rack
  info table one to three minutes.
</p>

{#if error}
  <p class="mb-4 text-destructive">{error}</p>
{/if}

{#if failed.length > 0}
  <div class="card mb-6 border-destructive p-4">
    <p class="font-medium text-destructive">
      {failed.length}
      {failed.length === 1 ? 'build has' : 'builds have'} given up.
    </p>
    <p class="mt-1 text-sm text-muted-foreground">
      A build is a pure function of its inputs, so a repeated failure is a missing input or a
      broken binary rather than bad luck — retrying without changing anything will fail the
      same way. The commonest cause is a lexicon imported before the server stored lexicon
      bytes: re-import that tarball, which adds no rows for files whose bytes have not
      changed, then retry.
    </p>
  </div>
{:else if waiting.length > 0}
  <p class="mb-6 text-sm">
    {waiting.length}
    {waiting.length === 1 ? 'file is' : 'files are'} still being built. Jobs that need them are
    waiting.
  </p>
{/if}

{#if rows.length === 0 && !error}
  <p class="text-muted-foreground">
    Nothing has been asked for yet. A build is queued when a job is created or activated whose
    players ask for a wordmap or a rack info table.
  </p>
{:else}
  <div class="card overflow-x-auto p-0">
    <table class="table">
      <thead>
        <tr>
          <th>File</th>
          <th>Name</th>
          <th>Builder</th>
          <th>State</th>
          <th class="text-right">Size</th>
          <th>SHA-256</th>
          <th>Requested</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each rows as row (row.role + row.name + row.builder)}
          <tr class:text-destructive={row.state === 'failed'}>
            <td>{kind(row.role)}</td>
            <td><code>{row.name}</code></td>
            <td>
              <code>{row.builder}</code>
              {#if row.build_target}
                <span class="text-muted-foreground">/ {row.build_target}</span>
              {/if}
            </td>
            <td>
              {row.state}
              {#if row.attempts > 1}
                <span class="text-muted-foreground">({row.attempts} attempts)</span>
              {/if}
              {#if row.error}
                <p class="mt-1 text-xs text-muted-foreground">{row.error}</p>
              {/if}
            </td>
            <td class="text-right">{mib(row.bytes)}</td>
            <td><code class="text-xs">{row.sha256 ? row.sha256.slice(0, 12) : '—'}</code></td>
            <td>{datetime(row.requested_at)}</td>
            <td>
              {#if row.state === 'failed'}
                <button
                  class="text-sm underline"
                  disabled={busy === `${row.role} ${row.name}`}
                  on:click={() => retry(row)}
                >
                  Retry
                </button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}
