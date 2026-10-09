<script lang="ts">
  /**
   * An opening-rack job's consensus settings, changed after creation: the
   * fewest and most analyses a rack gets, and the share of them that must
   * agree on its best move. Saving restates every rack, and the job follows:
   * a completed job with racks unsettled again reopens, and an active one
   * with every rack settled completes. A static player analyses each rack
   * once, so its job has nothing to change here. `saved` fires after a save,
   * for the page to read the job and its settings again.
   */
  import { createEventDispatcher } from 'svelte';
  import { api, errorText } from '$lib/api';
  import { consensusFields, consensusProblem } from '$lib/consensus';
  import type { JobConfig } from '$lib/jobSettings';

  export let jobId: string;
  export let config: JobConfig;
  /** Another action of the page's is running, or the job is gone. */
  export let disabled = false;

  const dispatch = createEventDispatcher<{ saved: void }>();

  $: settings = config.opening_racks!;
  $: simulates = config.players.some((p) => p.num_plies > 0);

  // Filled from the settings once, and again after each save: what the admin
  // types is not overwritten by a read in between.
  let minResults = 1;
  let maxResults = 1;
  let consensusPct = 80;
  let filledFrom: JobConfig | null = null;
  $: if (filledFrom !== config) {
    filledFrom = config;
    minResults = settings.min_results_per_rack;
    maxResults = settings.max_results_per_rack;
    consensusPct = settings.consensus_pct;
  }

  let busy = false;
  let error = '';
  let notice = '';

  $: typed = {
    min_results_per_rack: minResults,
    max_results_per_rack: maxResults,
    consensus_pct: consensusPct
  };
  $: problem = consensusProblem(typed);
  // What a save sends: no share at one analysis per rack, whose box is
  // disabled, so the job keeps its stored one.
  $: fields = consensusFields(typed);
  $: unchanged =
    fields.min_results_per_rack === settings.min_results_per_rack &&
    fields.max_results_per_rack === settings.max_results_per_rack &&
    (fields.consensus_pct === undefined || fields.consensus_pct === settings.consensus_pct);

  async function save() {
    if (busy || problem) return;
    busy = true;
    error = '';
    notice = '';
    try {
      const result = await api.updateConsensus(jobId, fields);
      const racks = `${result.unsettled_racks.toLocaleString()} rack${result.unsettled_racks === 1 ? '' : 's'} unsettled`;
      notice = result.reopened
        ? `Saved: ${racks}, so the job reopened, inactive at 0% — give it an allocation on the Allocation page to run it.`
        : result.job.status === 'completed'
          ? `Saved: the job is completed, with ${racks}.`
          : `Saved: ${racks}.`;
      dispatch('saved');
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="card space-y-4" data-testid="consensus-editor">
  <h2 class="text-lg font-medium">Consensus</h2>
  {#if !simulates}
    <p class="text-sm text-muted-foreground">
      Its player is static, so every analysis of a rack ranks it the same way: one analysis per
      rack, and no consensus to change.
    </p>
  {:else}
    <form class="space-y-3" on:submit|preventDefault={save}>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div>
          <label class="label" for="edit-minres">Minimum Analyses Per Rack</label>
          <input id="edit-minres" type="number" min="1" max="100" class="input" bind:value={minResults} />
        </div>
        <div>
          <label class="label" for="edit-maxres">Maximum Analyses Per Rack</label>
          <input id="edit-maxres" type="number" min="1" max="100" class="input" bind:value={maxResults} />
        </div>
        <!-- No `min`: the share must be above 50, which no `min` can say (51 blocked the
             50.5 the server takes); `consensusProblem` says what it must be. -->
        <div>
          <label class="label" for="edit-consensus">Consensus %</label>
          <input
            id="edit-consensus"
            type="number"
              max="100"
            step="any"
            class="input"
            bind:value={consensusPct}
            disabled={maxResults <= 1}
          />
        </div>
      </div>
      {#if problem}<p class="field-error">{problem}</p>{/if}
      <p class="text-xs text-muted-foreground">
        Saving restates every rack under the new settings. Raising them unsettles racks, which a
        completed job reopens to analyse again (inactive at 0%, until you give it an allocation);
        lowering them settles racks, and an active job with none left unsettled completes.
      </p>
      <button class="btn-primary" type="submit" disabled={disabled || busy || !!problem || unchanged}>
        Save
      </button>
    </form>
  {/if}
  <div aria-live="polite">
    {#if notice}<p class="text-sm" data-testid="consensus-notice">{notice}</p>{/if}
    {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  </div>
</div>
