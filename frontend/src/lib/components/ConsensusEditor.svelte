<script lang="ts">
  /**
   * An opening-rack job's consensus settings, changed after creation: the
   * most analyses a rack gets, the lead over the runner-up that settles it,
   * and how many of its first analysis's moves its later ones rank. Saving the
   * most or the lead restates every rack, and the job follows: a completed
   * job with racks unsettled again reopens, and an active one with every rack
   * settled completes. The moves reanalysed apply to the reissues sent from
   * then on. A static player analyses each rack once, so its job has nothing
   * to change here. `saved` fires after a save, for the page to read the job
   * and its settings again.
   */
  import { createEventDispatcher } from 'svelte';
  import { api, errorText } from '$lib/api';
  import { CANDIDATES_MIN_MAGPIE, consensusFields, consensusProblem, versionAtLeast } from '$lib/consensus';
  import type { JobConfig } from '$lib/jobSettings';

  export let jobId: string;
  export let config: JobConfig;
  /** Another action of the page's is running, or the job is gone. */
  export let disabled = false;

  const dispatch = createEventDispatcher<{ saved: void }>();

  $: settings = config.opening_racks!;
  $: simulates = config.players.some((p) => p.num_plies > 0);
  // A job analysing a rack more than once sends its later analyses' moves,
  // which only MAGPIE 0.1.5 and later read; the server refuses the save below it.
  $: floorTooOld = !versionAtLeast(config.job.min_magpie_version, CANDIDATES_MIN_MAGPIE);

  // Filled from the settings once, and again after each save: what the admin
  // types is not overwritten by a read in between.
  let maxResults = 1;
  let topMoves = 10;
  let winMargin = 2;
  let filledFrom: JobConfig | null = null;
  $: if (filledFrom !== config) {
    filledFrom = config;
    maxResults = settings.max_results_per_rack;
    topMoves = settings.reissue_top_moves;
    winMargin = settings.win_margin;
  }

  let busy = false;
  let error = '';
  let notice = '';

  $: fields = consensusFields({
    max_results_per_rack: maxResults,
    reissue_top_moves: topMoves,
    win_margin: winMargin
  });
  $: problem =
    consensusProblem(fields) ??
    (fields.max_results_per_rack > 1 && floorTooOld
      ? `Analysing a rack more than once needs MAGPIE ${CANDIDATES_MIN_MAGPIE} or later: raise the job's Oldest MAGPIE (now ${config.job.min_magpie_version}) in its Controls first.`
      : null);
  $: unchanged =
    fields.max_results_per_rack === settings.max_results_per_rack &&
    fields.reissue_top_moves === settings.reissue_top_moves &&
    fields.win_margin === settings.win_margin;

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
          <label class="label" for="edit-maxres">Max Passes Per Rack</label>
          <input id="edit-maxres" type="number" min="1" max="100" class="input" bind:value={maxResults} />
        </div>
        <div>
          <label class="label" for="edit-margin">Win By</label>
          <input
            id="edit-margin"
            type="number"
            min="1"
            max="100"
            class="input"
            bind:value={winMargin}
            disabled={maxResults <= 1}
          />
        </div>
        <div>
          <label class="label" for="edit-top">Reanalyse Top Moves</label>
          <input
            id="edit-top"
            type="number"
            min="2"
            max="100"
            class="input"
            bind:value={topMoves}
            disabled={maxResults <= 1}
          />
        </div>
      </div>
      {#if problem}<p class="field-error">{problem}</p>{/if}
      <p class="text-xs text-muted-foreground">
        A rack's first analysis ranks every play; each later one ranks only the first's top moves, and
        the rack settles once one move has been ranked first that many more times than any other (the
        first analysis counts), or after the most passes. Changing the most or the lead restates every
        rack: raising them unsettles racks, which a completed job reopens to analyse again (inactive at
        0%, until you give it an allocation); lowering them settles racks, and an active job with none
        left unsettled completes. Changing the moves reanalysed applies to the reanalyses sent from now on.
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
