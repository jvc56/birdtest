<script lang="ts">
  /**
   * The settings an admin changes at run time, without a deploy: for now the
   * task time limit. Every claim is given the limit as it stands when it is
   * made, so a change applies to the claims made after it, and a claim keeps
   * the deadline it was given.
   */
  import { onMount } from 'svelte';
  import { api, errorText, type Settings } from '$lib/api';
  import { computeTime, datetime } from '$lib/format';

  const MIN_SECONDS = 60;
  const MAX_SECONDS = 86_400;

  let settings: Settings | null = null;
  let maxTaskSeconds = 3600;
  let busy = false;
  let error = '';
  let saved = '';

  $: valid = Number.isInteger(maxTaskSeconds) && maxTaskSeconds >= MIN_SECONDS && maxTaskSeconds <= MAX_SECONDS;
  $: changed = settings !== null && maxTaskSeconds !== settings.max_task_seconds;

  onMount(async () => {
    try {
      settings = await api.settings();
      maxTaskSeconds = settings.max_task_seconds;
    } catch (e) {
      error = errorText(e);
    }
  });

  async function save() {
    error = '';
    saved = '';
    busy = true;
    try {
      settings = await api.updateSettings({ max_task_seconds: maxTaskSeconds });
      maxTaskSeconds = settings.max_task_seconds;
      saved = 'Saved. Claims made from now on are given the new limit.';
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<h1 class="mb-2 text-2xl font-semibold">Settings</h1>
<p class="mb-6 text-sm text-muted-foreground">
  What the server does that an admin may change at any time, without a deploy. Each change is in
  the audit log.
</p>

{#if !settings}
  {#if error}<p class="field-error" role="alert">{error}</p>{:else}<p class="text-muted-foreground">Loading…</p>{/if}
{:else}
  <form class="card max-w-xl space-y-4" on:submit|preventDefault={save} data-testid="settings">
    <div>
      <label class="label" for="max-task-seconds">Task Time Limit (Seconds)</label>
      <input
        id="max-task-seconds"
        type="number"
        min={MIN_SECONDS}
        max={MAX_SECONDS}
        step="1"
        class="input"
        bind:value={maxTaskSeconds}
      />
      <p class="mt-1 text-xs text-muted-foreground">
        {#if valid}
          {computeTime(maxTaskSeconds)}.
        {:else}
          <span class="field-error">A whole number of seconds from {MIN_SECONDS} to {MAX_SECONDS.toLocaleString()} (a minute to a day).</span>
        {/if}
        A worker stops a task that runs this long and hands it back, and a claim that runs a minute
        past it is taken back whether or not its worker still answers. A job's page counts its tasks
        that hit the limit; three in a row with none completed between set the job aside, since its
        batch is too big for the limit. A change applies to the claims made after it.
      </p>
    </div>
    <p class="text-xs text-muted-foreground">
      {#if settings.updated_by}
        Last changed by {settings.updated_by}, {datetime(settings.updated_at)}.
      {:else}
        Never changed: these are the defaults.
      {/if}
    </p>
    {#if error}<p class="field-error" role="alert">{error}</p>{/if}
    {#if saved}<p class="text-sm" role="status">{saved}</p>{/if}
    <button class="btn-primary" disabled={busy || !valid || !changed}>
      {busy ? 'Saving…' : 'Save'}
    </button>
  </form>
{/if}
