<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { optionalIntList, optionalNumber } from '$lib/format';
  import { api, errorText, type InputData } from '$lib/api';

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
  // MAGPIE's default. A static player's list is sized by it too: an opening-rack
  // job cannot report more plays per rack than this.
  let staticNumPlays = 100;
  let numPlaysRecorded = 10;
  let numPliesRecorded = 2;
  let stoppingPct = 99;
  // The server takes a stopping percentage strictly between 0 and 100, which
  // `min` and `max` cannot say: they let both ends through. The browser
  // holds the form back with this instead.
  let stoppingInput: HTMLInputElement | undefined;
  $: stoppingInput?.setCustomValidity(
    stoppingPct > 0 && stoppingPct < 100 ? '' : 'Stopping % must be above 0 and below 100.'
  );
  let useInference = false;
  let showAdvanced = false;
  // Exhaustive on purpose: any MAGPIE option not stated here falls back to
  // whatever a worker's own process happens to have, which can differ across
  // workers.
  let useWordmap = true;
  let useRit = true;
  // On by default, although MAGPIE has it opt-in (-wit): it speeds move
  // generation and costs little.
  let useWit = true;
  // Blank means "MAGPIE's default" (see optionalNumber).
  const optional = optionalNumber;
  let minPlayIterations: number | '' = '';
  let threshold = '';
  let samplingRule = '';
  let inferenceMargin: number | '' = '';
  let utilityWWinpct: number | '' = '';
  let utilityWSpread: number | '' = '';
  let utilitySpreadScale: number | '' = '';
  let movegenMargin: number | '' = '';
  // Endgame and pre-endgame solving, for games and game-pairs jobs. The
  // endgame is the switch for both: the pre-endgame scores its emptier
  // scenarios with endgame solves, so it cannot run without one.
  let solveEndgame = false;
  // MAGPIE's own `eplies` default. The server has none to fill in: the depth
  // is the endgame's switch (0 is off), so the form always states it.
  let endgamePlies = 6;
  let runPeg = false;
  // Pre-filled below MAGPIE's own 4: at 4, with the default schedule and
  // nested lookahead, one game took minutes on two threads.
  let pegMaxBag = 2;
  let showPegAdvanced = false;
  // Blank means MAGPIE's default, which the server writes into the config.
  let pegStageTopK = '';
  let pegScenarioStride: number | '' = '';
  let pegOppModel = '';
  let pegNested = true;
  let pegNestedCandCaps = '';
  let pegNestedMaxDepth: number | '' = '';
  let pegNestedStrides = '';
  $: if (!solveEndgame) runPeg = false;
  let error = '';
  // Whether `error` is the last submit's: only that is cleared by an edit, not
  // a failure to load the form's choices.
  let fromSubmit = false;
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
    try {
      files = await api.inputData();
    } catch (e) {
      error = `Could not load input data: ${errorText(e)}`;
      return;
    }
    // Filtered from `files` here rather than read off the `$:` arrays above:
    // those are recomputed on the update cycle, not on assignment, so they
    // would still be empty on the next line and every default would be ''.
    kwgId = firstOfRole('kwg');
    klvId = firstOfRole('klv');
  });

  /** A list field's numbers, or throws naming the field. */
  function listField(text: string, label: string): number[] | null {
    const parsed = optionalIntList(text);
    if ('error' in parsed) throw new Error(`${label}: ${parsed.error}`);
    return parsed.values;
  }

  async function submit() {
    busy = true;
    error = '';
    try {
      const peg = solveEndgame && runPeg;
      const nested = peg && pegNested;
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
        num_plays: simming ? numPlays : staticNumPlays,
        num_plays_recorded: numPlaysRecorded,
        num_plies_recorded: simming ? numPliesRecorded : null,
        stopping_pct: simming ? stoppingPct : null,
        use_inference: simming ? useInference : null,
        // Always 0 for a simmer: a time limit makes results depend on the
        // contributor's hardware, so the iteration budget bounds a simulation.
        time_limit_secs: simming ? 0 : null,
        use_wordmap: useWordmap,
        use_rit: useRit,
        use_wit: useWit,
        // Left blank, the server writes MAGPIE's default into the config. These
        // are simulation settings, which a static player states none of: the
        // server refuses one that does.
        min_play_iterations: simming ? optional(minPlayIterations) : null,
        threshold: (simming && threshold) || null,
        sampling_rule: (simming && samplingRule) || null,
        inference_margin: simming ? optional(inferenceMargin) : null,
        utility_w_winpct: simming ? optional(utilityWWinpct) : null,
        utility_w_spread: simming ? optional(utilityWSpread) : null,
        utility_spread_scale: simming ? optional(utilitySpreadScale) : null,
        movegen_margin: optional(movegenMargin),
        // 0 solves nothing. The pre-endgame's settings are stated only when it
        // runs, and the nested ones only with nested lookahead: the server
        // refuses a setting nothing would read.
        endgame_plies: solveEndgame ? endgamePlies : 0,
        peg_max_bag: peg ? pegMaxBag : 0,
        peg_stage_top_k: peg ? listField(pegStageTopK, 'PEG Schedule') : null,
        peg_scenario_stride: peg ? optional(pegScenarioStride) : null,
        peg_opp_model: (peg && pegOppModel) || null,
        peg_nested: peg ? pegNested : null,
        peg_nested_cand_caps: nested ? listField(pegNestedCandCaps, 'Nested Caps') : null,
        peg_nested_max_depth: nested ? optional(pegNestedMaxDepth) : null,
        peg_nested_strides: nested ? listField(pegNestedStrides, 'Nested Strides') : null
      });
      goto('/admin/player-configs');
      return created;
    } catch (e) {
      error = errorText(e);
      fromSubmit = true;
    } finally {
      busy = false;
    }
  }
</script>

<h1 class="mb-6 text-2xl font-semibold">New player config</h1>

<form class="card max-w-2xl space-y-4" on:submit|preventDefault={submit} on:input={() => { if (fromSubmit) { error = ''; fromSubmit = false; } }}>
  <div>
    <label class="label" for="name">Name</label>
    <input id="name" class="input" bind:value={name} placeholder="simmer-NWL23-4ply" maxlength="100" required />
  </div>

  <div class="grid grid-cols-2 gap-3">
    <div>
      <label class="label" for="recorder">Move Recorder (-r)</label>
      <select id="recorder" class="input" bind:value={recorderType}>
        <option value="best">best — keep only the top-ranked move (right for games jobs)</option>
        <option value="equity">equity — keep every move within the equity margin</option>
        <option value="all">all — keep every move</option>
      </select>
      <p class="mt-1 text-xs text-muted-foreground">
        This decides what move generation <em>keeps</em>, not which move is played.
        <strong>best</strong> throws away every candidate but the winner, so a static config
        using it can only ever report one move per position. A simmer is unaffected: it ranks
        every play up to its number of plays, whatever the recorder. Right for games jobs, where
        only the move played matters. A static opening-rack player wants a ranking, so it needs
        <strong>all</strong> unless it records exactly one play. (<strong>equity</strong> keeps only
        the moves within the equity margin of the best, which can be fewer than it reports.)
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
        <label class="label" for="winpct">Win % Model (-winpct)</label>
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
    <label class="label" for="npres">Moves Recorded (maxnumdplays)</label>
    <input id="npres" type="number" min="1" required class="input" bind:value={numPlaysRecorded} />
    <p class="mt-1 text-xs text-muted-foreground">
      How many ranked plays birdtest stores per analysed position. Separate from
      how many are generated or simulated, and never more than that: an
      opening-rack job refuses a config that reports more plays than it generates.
    </p>
  </div>

  {#if !simming}
    <div>
      <label class="label" for="nps">Moves Generated (-np)</label>
      <input id="nps" type="number" min="1" required class="input" bind:value={staticNumPlays} />
      <p class="mt-1 text-xs text-muted-foreground">
        How many plays move generation keeps. An opening-rack analysis ranks these, so
        it must be at least the plays to report.
      </p>
    </div>
  {/if}

  <label class="flex items-center gap-2 text-sm">
    <input type="checkbox" bind:checked={simming} />
    Simming Player
  </label>

  {#if simming}
    <div class="grid grid-cols-2 gap-3">
      <div><label class="label" for="iters">Maximum Total Iterations (-i)</label><input id="iters" type="number" class="input" bind:value={maxIterations} /></div>
      <div><label class="label" for="num_plies">Plies (-pl)</label><input id="num_plies" type="number" class="input" bind:value={numPlies} /></div>
      <div><label class="label" for="np">Moves Generated (-np)</label><input id="np" type="number" class="input" bind:value={numPlays} /></div>
      <div><label class="label" for="npr">Plies Recorded (shplies)</label><input id="npr" type="number" class="input" bind:value={numPliesRecorded} /></div>
      <div><label class="label" for="sc">Stopping % (-sc)</label><input id="sc" type="number" step="any" min="0" max="100" class="input" bind:this={stoppingInput} bind:value={stoppingPct} /></div>
      <p class="text-sm">No time limit: a simulation stops at its iteration budget, so what it finds does not depend on the contributor's hardware.</p>
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useInference} />
        Uses Inference (-si)
      </label>
    </div>
  {:else}
    <div>
      <label class="label" for="sort">Sorted By (-s)</label>
      <select id="sort" class="input" bind:value={sortStrategy}>
        <option value="equity">equity — score plus leave value (standard static player)</option>
        <option value="score">score — raw score only (static players; a simmer ranks by equity)</option>
      </select>
    </div>
  {/if}

  <fieldset class="space-y-3 rounded border p-3">
    <legend class="px-1 text-sm font-medium">Endgame and Preendgame</legend>
    <p class="text-xs text-muted-foreground">
      For games and game-pairs jobs; an opening-rack job never reaches a small bag, and a
      leave job refuses a player that solves. No time limit applies: the depth and the
      schedule bound the work, so the player is as strong on a slow machine as a fast one.
    </p>
    <div class="grid grid-cols-2 gap-3">
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={solveEndgame} />
        Uses Endgame
      </label>
      {#if solveEndgame}
        <div>
          <label class="label" for="eplies">Endgame Plies (-eplies1)</label>
          <input id="eplies" type="number" min="1" max="25" required class="input" bind:value={endgamePlies} />
        </div>
      {/if}
      <label class="flex items-end gap-2 text-sm" title={solveEndgame ? '' : 'The pre-endgame needs endgame solving'}>
        <input type="checkbox" bind:checked={runPeg} disabled={!solveEndgame} />
        Uses Preendgame
      </label>
      {#if runPeg}
        <div>
          <label class="label" for="pegbag">Preendgame Maximum Bag (-pegbag1)</label>
          <input id="pegbag" type="number" min="1" max="4" required class="input" bind:value={pegMaxBag} />
        </div>
      {/if}
    </div>
    {#if runPeg}
      <p class="text-xs text-muted-foreground">
        Cost grows steeply with the bag and the schedule: at a bag of 4 with the default
        schedule and nested lookahead, a single game can take minutes. Prefer a smaller
        games-per-task for jobs with this player.
      </p>
      <button type="button" class="text-sm text-muted-foreground underline" on:click={() => (showPegAdvanced = !showPegAdvanced)}>
        {showPegAdvanced ? 'Hide' : 'Show'} pre-endgame schedule
      </button>
      {#if showPegAdvanced}
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="label" for="pegtopk">PEG Schedule (-pegtopk1)</label>
            <input id="pegtopk" class="input" bind:value={pegStageTopK} placeholder="32, 16, 8, 4, 2" />
          </div>
          <div>
            <label class="label" for="pegstride">PEG Stride (-pegstride1)</label>
            <input id="pegstride" type="number" min="1" class="input" bind:value={pegScenarioStride} placeholder="1" />
          </div>
          <div>
            <label class="label" for="pegopp">PEG Opponent (-pegpess1)</label>
            <select id="pegopp" class="input" bind:value={pegOppModel}>
              <option value="">MAGPIE default (rational)</option>
              <option value="rational">rational</option>
              <option value="pessimistic">pessimistic</option>
            </select>
          </div>
          <label class="flex items-end gap-2 text-sm">
            <input type="checkbox" bind:checked={pegNested} />
            Nested Lookahead (-pegnested1)
          </label>
          {#if pegNested}
            <div>
              <label class="label" for="pegncaps">Nested Caps (-pegncaps)</label>
              <input id="pegncaps" class="input" bind:value={pegNestedCandCaps} placeholder="8, 4, 2" />
            </div>
            <div>
              <label class="label" for="pegndepth">Nested Depth (-pegndepth)</label>
              <input id="pegndepth" type="number" min="1" max="4" class="input" bind:value={pegNestedMaxDepth} placeholder="1" />
            </div>
            <div>
              <label class="label" for="pegnstrides">Nested Strides, Bag 1–4 (-pegnstrides)</label>
              <input id="pegnstrides" class="input" bind:value={pegNestedStrides} placeholder="1, 1, 5, 7" />
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </fieldset>

  <button type="button" class="text-sm text-muted-foreground underline" on:click={() => (showAdvanced = !showAdvanced)}>
    {showAdvanced ? 'Hide' : 'Show'} advanced options
  </button>

  {#if showAdvanced}
    <div class="grid grid-cols-2 gap-3 rounded border p-3">
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useWordmap} />
        Wordmap (-w)
      </label>
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useRit} />
        Rack Info Table (-rit)
      </label>
      {#if useRit}
        <p class="col-span-2 text-sm text-muted-foreground">
          The server builds this player's table from its lexicon and leaves before any
          job using it can dispatch — a few minutes, once per pair. Watch it at
          <a class="underline" href="/admin/derived-data">derived data</a>. Contributors
          build their own copy, which costs about 1.9&nbsp;GB of disk and of memory (shared by
          workers on one machine).
        </p>
      {/if}
      <label class="flex items-end gap-2 text-sm">
        <input type="checkbox" bind:checked={useWit} />
        Word Info Table (-wit)
      </label>
      {#if useWit}
        <p class="col-span-2 text-sm text-muted-foreground">
          A letter mask move generation prunes with, built from the lexicon alone. The server
          builds it before any job using it can dispatch — a few seconds, once per lexicon.
          Contributors build their own copy, about 122&nbsp;MB for CSW24.
        </p>
      {/if}
      <div>
        <label class="label" for="minpi">Minimum Iterations per Play (-mi)</label>
        <input id="minpi" type="number" class="input" bind:value={minPlayIterations} />
      </div>
      <div>
        <label class="label" for="threshold">Stopping Threshold Rule (-th)</label>
        <select id="threshold" class="input" bind:value={threshold}>
          <option value="">MAGPIE default</option>
          <option value="none">none</option>
          <option value="gk16">gk16</option>
        </select>
      </div>
      <div>
        <label class="label" for="samplingrule">Sampling Rule (-sa)</label>
        <select id="samplingrule" class="input" bind:value={samplingRule}>
          <option value="">MAGPIE default</option>
          <option value="round_robin">round_robin</option>
          <option value="top_two_ids">top_two_ids</option>
        </select>
      </div>
      <div>
        <label class="label" for="im">Inference Margin (-im)</label>
        <input id="im" type="number" step="any" class="input" bind:value={inferenceMargin} />
      </div>
      <div>
        <label class="label" for="uwin">Win % Utility Weight (-uwin)</label>
        <input id="uwin" type="number" step="any" class="input" bind:value={utilityWWinpct} />
      </div>
      <div>
        <label class="label" for="uspread">Spread Utility Weight (-uspread)</label>
        <input id="uspread" type="number" step="any" class="input" bind:value={utilityWSpread} />
      </div>
      <div>
        <label class="label" for="uspreadscale">Spread Utility Scale (-uspreadscale)</label>
        <input id="uspreadscale" type="number" step="any" class="input" bind:value={utilitySpreadScale} />
      </div>
      <div>
        <label class="label" for="mmargin">Movegen Margin (-mmargin)</label>
        <input id="mmargin" type="number" step="any" class="input" bind:value={movegenMargin} />
        <p class="mt-1 text-xs text-muted-foreground">
          Used only by an opening-rack job with the equity recorder; games play with a margin of 0.
        </p>
      </div>
    </div>
  {/if}

  <!-- Announced: an error that appears after a submit is otherwise silent to a
       screen reader. -->
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create'}</button>
</form>
