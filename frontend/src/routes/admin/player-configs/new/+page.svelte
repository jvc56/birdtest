<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api, type InputData } from '$lib/api';

  let name = '';
  let recorderType = 'best';
  let sortStrategy: string = 'equity';
  // Files are picked from imported rows rather than typed: a config pins exact
  // bytes, which is what lets a worker be checked against them.
  let files: InputData[] = [];
  let kwgId = '';
  let klvId = '';
  let winpctId = '';
  let simming = false;
  let maxIterations = 1000;
  let numPlies = 2;
  let numPlays = 10;
  let numPlaysRecorded = 10;
  let numPliesRecorded = 2;
  let stoppingPct = 99;
  let useInference = false;
  let showAdvanced = false;
  // Exhaustive on purpose: any MAGPIE option not stated here falls back to
  // whatever a worker's own process happens to have, which can differ across
  // workers.
  let useWordmap = true;
  let useRit = false;
  let minPlayIterations: number | '' = '';
  let threshold = '';
  let samplingRule = '';
  let inferenceMargin: number | '' = '';
  let utilityWWinpct: number | '' = '';
  let utilityWSpread: number | '' = '';
  let utilitySpreadScale: number | '' = '';
  let movegenMargin: number | '' = '';
  let error = '';
  let busy = false;

  $: lexica = files.filter((f) => f.role === 'kwg');
  $: leaves = files.filter((f) => f.role === 'klv');
  $: winpcts = files.filter((f) => f.role === 'winpct');
  // A simming player loads a win% model; a static one never opens it.
  $: if (!simming) winpctId = '';

  function label(file: InputData): string {
    return `${file.name} (${file.tarball_date}, ${file.sha256.slice(0, 8)})`;
  }

  function firstOfRole(role: string): string {
    return files.find((f) => f.role === role)?.id ?? '';
  }

  onMount(async () => {
    files = await api.inputData();
    // Filtered from `files` here rather than read off the `$:` arrays above:
    // those are recomputed on the update cycle, not on assignment, so they
    // would still be empty on the next line and every default would be ''.
    kwgId = firstOfRole('kwg');
    klvId = firstOfRole('klv');
  });

  async function submit() {
    busy = true;
    error = '';
    try {
      const created = await api.createPlayerConfig({
        name,
        recorder_type: recorderType,
        // A simming player's move comes from the simulation, not a static sort.
        sort_strategy: simming ? null : sortStrategy,
        kwg_id: kwgId,
        klv_id: klvId,
        winpct_id: simming ? winpctId || null : null,
        max_iterations: simming ? maxIterations : null,
        num_plies: simming ? numPlies : null,
        num_plays: simming ? numPlays : null,
        num_plays_recorded: numPlaysRecorded,
        num_plies_recorded: simming ? numPliesRecorded : null,
        stopping_pct: simming ? stoppingPct : null,
        use_inference: simming ? useInference : null,
        // Always 0 for a simmer: a time limit makes results depend on the
        // contributor's hardware, so the iteration budget bounds a simulation.
        time_limit_secs: simming ? 0 : null,
        use_wordmap: useWordmap,
        use_rit: useRit,
        min_play_iterations: minPlayIterations === '' ? null : Number(minPlayIterations),
        threshold: threshold || null,
        sampling_rule: samplingRule || null,
        inference_margin: inferenceMargin === '' ? null : Number(inferenceMargin),
        utility_w_winpct: utilityWWinpct === '' ? null : Number(utilityWWinpct),
        utility_w_spread: utilityWSpread === '' ? null : Number(utilityWSpread),
        utility_spread_scale: utilitySpreadScale === '' ? null : Number(utilitySpreadScale),
        movegen_margin: movegenMargin === '' ? null : Number(movegenMargin)
      });
      goto('/admin/player-configs');
      return created;
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<h1 class="mb-6 text-2xl font-semibold">New player config</h1>

<form class="card max-w-2xl space-y-4" on:submit|preventDefault={submit}>
  <div>
    <label class="label" for="name">Name</label>
    <input id="name" class="input" bind:value={name} placeholder="simmer-NWL23-4ply" required />
  </div>

  <div class="grid grid-cols-2 gap-3">
    <div>
      <label class="label" for="recorder">Recorder type (-r)</label>
      <select id="recorder" class="input" bind:value={recorderType}>
        <option value="best">best — keep only the top-ranked move (right for games jobs)</option>
        <option value="equity">equity — keep every move within the equity margin</option>
        <option value="all">all — keep every move</option>
      </select>
      <p class="mt-1 text-xs text-muted-foreground">
        This decides what move generation <em>keeps</em>, not which move is played.
        <strong>best</strong> throws away every candidate but the winner, so a config using it
        can only ever report one move per position — and a simmer using it has nothing to choose
        between, which makes plies and plays do nothing. Right for games jobs, where only the
        move played matters. An opening-rack job wants a ranking, so it needs
        <strong>all</strong> or <strong>equity</strong> unless it records exactly one play.
      </p>
    </div>
    <div>
      <label class="label" for="kwg">Lexicon (-l)</label>
      <select id="kwg" class="input" bind:value={kwgId} required>
        {#each lexica as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
    <div>
      <label class="label" for="klv">Leaves (-k)</label>
      <select id="klv" class="input" bind:value={klvId} required>
        {#each leaves as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
    {#if simming}
      <div>
        <label class="label" for="winpct">Win% model (-winpct)</label>
        <select id="winpct" class="input" bind:value={winpctId} required>
          <option value="">choose one</option>
          {#each winpcts as file}<option value={file.id}>{label(file)}</option>{/each}
        </select>
      </div>
    {/if}
  </div>
  <p class="text-xs text-muted-foreground">
    Files are pinned by content, not by name: a config names exact bytes, and a
    worker whose copy does not match declines the task rather than contributing
    something incomparable. New data means a new config — clone this one once
    the newer files are imported. A static player must not name a win% model,
    because MAGPIE never loads one for it.
  </p>

  <div>
    <label class="label" for="npres">Plays to report (maxnumdplays)</label>
    <input id="npres" type="number" min="1" required class="input" bind:value={numPlaysRecorded} />
    <p class="mt-1 text-xs text-muted-foreground">
      How many ranked plays birdtest stores per analysed position. Separate from
      how many are generated or simulated.
    </p>
  </div>

  <label class="flex items-center gap-2 text-sm">
    <input type="checkbox" bind:checked={simming} />
    Simming player
  </label>

  {#if simming}
    <div class="grid grid-cols-2 gap-3">
      <div><label class="label" for="iters">Max iterations (-i)</label><input id="iters" type="number" class="input" bind:value={maxIterations} /></div>
      <div><label class="label" for="num_plies">Plies (-pl)</label><input id="num_plies" type="number" class="input" bind:value={numPlies} /></div>
      <div><label class="label" for="np">Plays to simulate (-np)</label><input id="np" type="number" class="input" bind:value={numPlays} /></div>
      <div><label class="label" for="npr">Plies to report (shplies)</label><input id="npr" type="number" class="input" bind:value={numPliesRecorded} /></div>
      <div><label class="label" for="sc">Stopping % (-sc)</label><input id="sc" type="number" step="0.1" class="input" bind:value={stoppingPct} /></div>
      <p class="text-sm">No time limit: a simulation stops at its iteration budget, so what it finds does not depend on the contributor's hardware.</p>
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useInference} />
        Use inference (-si)
      </label>
    </div>
  {:else}
    <div>
      <label class="label" for="sort">Sort strategy (-s)</label>
      <select id="sort" class="input" bind:value={sortStrategy}>
        <option value="equity">equity — score plus leave value (standard static player)</option>
        <option value="score">score — raw score only</option>
      </select>
    </div>
  {/if}

  <button type="button" class="text-sm text-muted-foreground underline" on:click={() => (showAdvanced = !showAdvanced)}>
    {showAdvanced ? 'Hide' : 'Show'} advanced options
  </button>

  {#if showAdvanced}
    <div class="grid grid-cols-2 gap-3 rounded border p-3">
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useWordmap} />
        Use wordmap (-w)
      </label>
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useRit} />
        Use rack info table (-rit)
      </label>
      <div>
        <label class="label" for="minpi">Min play iterations (-mi)</label>
        <input id="minpi" type="number" class="input" bind:value={minPlayIterations} />
      </div>
      <div>
        <label class="label" for="threshold">Threshold (-th)</label>
        <select id="threshold" class="input" bind:value={threshold}>
          <option value="">MAGPIE default</option>
          <option value="none">none</option>
          <option value="gk16">gk16</option>
        </select>
      </div>
      <div>
        <label class="label" for="samplingrule">Sampling rule (-sa)</label>
        <select id="samplingrule" class="input" bind:value={samplingRule}>
          <option value="">MAGPIE default</option>
          <option value="round_robin">round_robin</option>
          <option value="top_two_ids">top_two_ids</option>
        </select>
      </div>
      <div>
        <label class="label" for="im">Inference margin (-im)</label>
        <input id="im" type="number" step="0.1" class="input" bind:value={inferenceMargin} />
      </div>
      <div>
        <label class="label" for="uwin">Utility weight: win% (-uwin)</label>
        <input id="uwin" type="number" step="0.1" class="input" bind:value={utilityWWinpct} />
      </div>
      <div>
        <label class="label" for="uspread">Utility weight: spread (-uspread)</label>
        <input id="uspread" type="number" step="0.1" class="input" bind:value={utilityWSpread} />
      </div>
      <div>
        <label class="label" for="uspreadscale">Utility spread scale (-uspreadscale)</label>
        <input id="uspreadscale" type="number" step="0.1" class="input" bind:value={utilitySpreadScale} />
      </div>
      <div>
        <label class="label" for="mmargin">Move-gen equity margin (-mmargin)</label>
        <input id="mmargin" type="number" step="0.1" class="input" bind:value={movegenMargin} />
        <p class="mt-1 text-xs text-muted-foreground">
          Shared across both players in a job, same as win% model.
        </p>
      </div>
    </div>
  {/if}

  {#if error}<p class="field-error">{error}</p>{/if}
  <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create'}</button>
</form>
