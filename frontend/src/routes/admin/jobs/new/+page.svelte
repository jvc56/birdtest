<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api, errorText, type InputData, type JobType, type PlayerConfig } from '$lib/api';
  import { blankFields, jobTypeLabel, leavePlayerConflict, parseTargetRackCounts, targetsText, unchosenText } from '$lib/format';
  import { consensusFields, consensusProblem as checkConsensus } from '$lib/consensus';

  let configs: PlayerConfig[] = [];
  let files: InputData[] = [];
  let error = '';
  // Whether `error` is the last submit's: only that is cleared by an edit, not
  // a failure to load the form's choices.
  let fromSubmit = false;
  let busy = false;

  let jobType: JobType = 'game_pairs';
  // Shown first wherever jobs are listed, and as the job page's title.
  let name = '';
  // Pre-filled from the server-wide floor rather than left blank: a default
  // nobody sees is how every new job quietly inherits a floor that is too low.
  let minMagpieVersion = '';
  let serverFloor = '';
  let variant = 'classic';
  let letterdistId = '';
  let layoutId = '';
  // Run-wide rules every request states. MAGPIE's defaults, which the server
  // writes in when the form leaves them as they are.
  let bingoBonus = 50;
  let simCutoff = 0.005;

  $: letterdists = files.filter((f) => f.role === 'letterdist');
  $: layouts = files.filter((f) => f.role === 'layout');

  function label(file: InputData): string {
    return `${file.name} (${file.tarball_date}, ${file.sha256.slice(0, 8)})`;
  }

  // Per-type fields. Only the ones the selected type uses are submitted.
  let playerConfigId = '';
  let player1 = '';
  let player2 = '';
  let batchSize = 1;
  // Off by default: without a test the job plays its games and stops, and the
  // test's settings are not sent (the server refuses them without the flag).
  let testEnabled = false;
  let minUnits = 1000;
  let maxUnits = 40000;
  let confidencePct = 95;
  // Keep every position the games analyse, searchable on the job's page.
  // What a games or pairs job's counts are of, for its labels.
  $: units = jobType === 'games' ? 'Games' : 'Pairs';
  let capturePositions = false;
  // Game pairs, with positions saved: only each pair's first divergence.
  let captureFirstDivergence = false;
  let numIterations = 10000;
  // One occurrence target per generation, as MAGPIE's `leavegen` takes them:
  // the list's length is how many generations the job runs.
  let targetRackCounts = '500';
  $: targets = parseTargetRackCounts(targetRackCounts);
  let racksPerTask = 50;
  // An opening-rack job's consensus: one analysis per rack unless asked.
  let minResults = 1;
  let maxResults = 1;
  let consensusPct = 80;

  const types: JobType[] = ['opening_rack', 'games', 'game_pairs', 'leave_generation'];

  // The two combinations job creation refuses, surfaced before the submit
  // rather than as the error that comes back from it. `-r best` is
  // MOVE_RECORD_BEST, so a static player's movegen keeps one play and the rest
  // of the ranking never exists (a simmer ranks every play up to num_plays
  // whatever its recorder). And no player reports more plays than num_plays,
  // which sizes the move list.
  $: selectedConfig = configs.find((config) => config.id === playerConfigId);
  $: selectedStatic = selectedConfig ? selectedConfig.num_plies === 0 : false;
  $: consensusProblem =
    jobType !== 'opening_rack' || selectedStatic
      ? null
      : checkConsensus({
          min_results_per_rack: minResults,
          max_results_per_rack: maxResults,
          consensus_pct: consensusPct
        });
  $: openingRackConflict =
    jobType !== 'opening_rack' || !selectedConfig
      ? null
      : selectedConfig.recorder_type === 'best' &&
          selectedConfig.num_plays_recorded > 1 &&
          selectedConfig.num_plies === 0
        ? `${selectedConfig.name} is static and records only the best move, so this job would store one play per rack rather than the ${selectedConfig.num_plays_recorded} it asks for.`
        : selectedConfig.num_plays < selectedConfig.num_plays_recorded
          ? `${selectedConfig.name} generates ${selectedConfig.num_plays} plays, so this job would store at most that many per rack rather than the ${selectedConfig.num_plays_recorded} it asks for.`
          : null;
  // A leave job's player plays statically on equity with no rack info table.
  $: leaveConflict =
    jobType !== 'leave_generation' || !selectedConfig ? null : leavePlayerConflict(selectedConfig);

  onMount(async () => {
    try {
      await loadChoices();
    } catch (e) {
      error = `Could not load player configs and input data: ${e instanceof Error ? e.message : String(e)}`;
    }
  });

  async function loadChoices() {
    [configs, files] = await Promise.all([api.playerConfigs(), api.inputData()]);
    if (configs.length) {
      playerConfigId = configs[0].id;
      player1 = configs[0].id;
      player2 = configs[configs.length - 1].id;
    }
    // The letter distribution and board are left for the admin to choose: the
    // first of each was whichever was imported first, and a job made on it
    // unnoticed played with the wrong bag or board.
    serverFloor = (await api.clientVersion()).min_magpie_version;
    minMagpieVersion = serverFloor;
  }

  function body(): Record<string, unknown> {
    const common = {
      name: name.trim(),
      job_type: jobType,
      variant,
      letterdist_id: letterdistId,
      layout_id: layoutId,
      bingo_bonus: bingoBonus,
      // A leave job's bot never simulates, so it states no cutoff: the server
      // refuses one.
      ...(jobType === 'leave_generation' ? {} : { sim_cutoff: simCutoff }),
      ...(minMagpieVersion ? { min_magpie_version: minMagpieVersion } : {})
    };
    const test = testEnabled
      ? { test_enabled: true, confidence_pct: confidencePct }
      : { test_enabled: false };
    switch (jobType) {
      case 'opening_rack':
        return {
          ...common,
          player_config_id: playerConfigId,
          // A static player's analyses always agree, so it gets one per rack.
          ...(selectedStatic
            ? { min_results_per_rack: 1, max_results_per_rack: 1 }
            : consensusFields({
                min_results_per_rack: minResults,
                max_results_per_rack: maxResults,
                consensus_pct: consensusPct
              }))
        };
      case 'games':
        return {
          ...common,
          player1_config_id: player1, player2_config_id: player2,
          games_per_batch: batchSize, max_games: maxUnits, ...test,
          ...(testEnabled ? { min_games: minUnits } : {}),
          capture_positions: capturePositions
        };
      case 'game_pairs':
        return {
          ...common,
          player1_config_id: player1, player2_config_id: player2,
          pairs_per_batch: batchSize, max_pairs: maxUnits, ...test,
          ...(testEnabled ? { min_pairs: minUnits } : {}),
          capture_positions: capturePositions,
          capture_first_divergence: capturePositions && captureFirstDivergence
        };
      case 'leave_generation':
        return {
          ...common,
          player_config_id: playerConfigId,
          num_iterations: numIterations,
          target_rack_counts: 'targets' in targets ? targets.targets : [],
          racks_per_task: racksPerTask
        };
    }
  }

  async function submit() {
    error = '';
    // A cleared number box binds as null, and the server's answer to a null
    // setting names the whole request ("data did not match any variant"),
    // not the field.
    // The browser holds a form whose required select is on its empty choice;
    // this names it should one get past, rather than sending an empty id.
    const unchosen = unchosenText({
      'a letter distribution': letterdistId,
      'a board layout': layoutId
    });
    if (unchosen) {
      error = unchosen;
      fromSubmit = true;
      return;
    }
    if (jobType === 'leave_generation' && 'error' in targets) {
      error = `Targets per generation: ${targets.error}`;
      fromSubmit = true;
      return;
    }
    if (consensusProblem) {
      error = `Analyses per rack: ${consensusProblem}`;
      fromSubmit = true;
      return;
    }
    const request = body();
    const blank = blankFields(request);
    if (blank.length) {
      error = `Fill in every setting: ${blank.join(', ')} is empty.`;
      fromSubmit = true;
      return;
    }
    busy = true;
    try {
      const created = await api.createJob(request);
      goto(`/admin/jobs/${created.job.id}`);
    } catch (e) {
      error = errorText(e);
      fromSubmit = true;
    } finally {
      busy = false;
    }
  }
</script>

<h1 class="mb-2 text-2xl font-semibold">Create a job</h1>
<p class="mb-6 text-sm text-muted-foreground">
  Jobs are created inactive. You set the allocation when you activate one, so you can review the
  whole active set first.
</p>

<!-- Any edit clears the last server error: a submit the browser blocks never
     reaches submit(), which is where it was cleared, so it lingered. -->
<form class="card max-w-2xl space-y-4" on:submit|preventDefault={submit} on:input={() => { if (fromSubmit) { error = ''; fromSubmit = false; } }}>
  <div>
    <label class="label" for="name">Job Name</label>
    <input
      id="name"
      class="input"
      bind:value={name}
      required
      maxlength="100"
      placeholder="e.g. simmer 4-ply vs static, NWL23"
    />
  </div>
  <div>
    <label class="label" for="type">Job Type</label>
    <select
      id="type"
      class="input"
      bind:value={jobType}
      on:change={() => {
        // A games batch is even (see the field below).
        if (jobType === 'games' && batchSize % 2 !== 0) batchSize += 1;
      }}
    >
      {#each types as type}<option value={type}>{jobTypeLabel(type)}</option>{/each}
    </select>
  </div>

  <div class="grid grid-cols-2 gap-3">
    <div>
      <label class="label" for="magpie">Oldest MAGPIE</label>
      <input id="magpie" class="input" bind:value={minMagpieVersion} placeholder="1.4.0" />
      <p class="mt-1 text-xs text-muted-foreground">
        Server-wide floor: {serverFloor || '—'}. Workers below this are never offered the job;
        raise it per job when the work needs a newer MAGPIE.
      </p>
    </div>
  </div>

  <div class="grid grid-cols-3 gap-3">
    <div>
      <label class="label" for="variant">Variant</label>
      <select id="variant" class="input" bind:value={variant}>
        <option value="classic">classic</option>
        <option value="wordsmog">wordsmog</option>
      </select>
    </div>
    <div>
      <label class="label" for="ld">Letter Distribution</label>
      <select id="ld" class="input" bind:value={letterdistId} required>
        <option value="" disabled selected>Choose…</option>
        {#each letterdists as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
    <div>
      <label class="label" for="layout">Board</label>
      <select id="layout" class="input" bind:value={layoutId} required>
        <option value="" disabled selected>Choose…</option>
        {#each layouts as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
  </div>
  <p class="text-xs text-muted-foreground">
    One distribution and one board per job: MAGPIE takes a single bag and board for the whole game.
    Each player's lexicon and leaves come from its own config.
  </p>

  <div class="grid grid-cols-2 gap-3">
    <div>
      <label class="label" for="bingo">Bingo Bonus (-bb)</label>
      <input id="bingo" type="number" min="0" max="500" step="1" required class="input" bind:value={bingoBonus} />
    </div>
    {#if jobType !== 'leave_generation'}
      <div>
        <label class="label" for="cutoff">Sim Cutoff (-cutoff)</label>
        <input id="cutoff" type="number" min="0" max="100" step="any" required class="input" bind:value={simCutoff} />
        <p class="mt-1 text-xs text-muted-foreground">
          How close to 0% or 100% two plays' win percentages must be for a simulation to treat
          them as equal and rank them by equity instead.
        </p>
      </div>
    {/if}
  </div>

  {#if jobType === 'opening_rack' || jobType === 'leave_generation'}
    <!-- One player, in both cases: an opening-rack job's analyses every rack,
         a leave job's plays both seats of every game. -->
    <div>
      <label class="label" for="pc">Player Config</label>
      <select id="pc" class="input" bind:value={playerConfigId} required>
        {#each configs as config}
          <option value={config.id}>
            {#if jobType === 'opening_rack'}
              {config.name} — recorder {config.recorder_type}, {config.num_plays_recorded} play{config.num_plays_recorded === 1
                ? ''
                : 's'} recorded
            {:else}
              {config.name} — {config.num_plies > 0 ? `${config.num_plies}-ply sim` : 'static'}, by
              {config.sort_strategy}{config.use_rit ? ', rack info table' : ''}{config.use_wordmap
                ? ', wordmap'
                : ''}{config.use_wit ? ', word info table' : ''}
            {/if}
          </option>
        {/each}
      </select>
    </div>
  {/if}

  {#if jobType === 'opening_rack'}
    {#if openingRackConflict}
      <p class="field-error">
        {openingRackConflict} Pick a static config whose recorder is <strong>all</strong>
        and that generates at least as many plays as it records, a simulating one that
        does, or one that records a single play.
      </p>
    {/if}
    <fieldset class="space-y-2" data-testid="consensus">
      <legend class="label">Analyses Per Rack</legend>
      {#if selectedStatic}
        <p class="text-xs text-muted-foreground">
          One: {selectedConfig?.name} is static, so every analysis of a rack ranks it the same
          way and there is no consensus to seek. Pick a simulating player to analyse each rack
          until its analyses agree.
        </p>
      {:else}
        <div class="grid grid-cols-3 gap-3">
          <div>
            <label class="label" for="minres">Minimum Analyses Per Rack</label>
            <input id="minres" type="number" min="1" max="100" class="input" bind:value={minResults} />
          </div>
          <div>
            <label class="label" for="maxres">Maximum Analyses Per Rack</label>
            <input id="maxres" type="number" min="1" max="100" class="input" bind:value={maxResults} />
          </div>
          <!-- No `min`: the share must be above 50, which no `min` can say (51 blocked the
               50.5 the server takes); `consensusProblem` says what it must be. -->
          <div>
            <label class="label" for="consensus">Consensus %</label>
            <input
              id="consensus"
              type="number"
                max="100"
              step="any"
              class="input"
              bind:value={consensusPct}
              disabled={maxResults <= 1}
            />
          </div>
        </div>
        {#if consensusProblem}<p class="field-error">{consensusProblem}</p>{/if}
        <p class="text-xs text-muted-foreground">
          {#if maxResults <= 1}
            One analysis per rack. Raise <strong>Maximum Analyses Per Rack</strong> to analyse a rack until its
            analyses agree.
          {:else}
            Each rack is analysed at least {minResults} time{minResults === 1 ? '' : 's'}, and again
            — by other workers where there are any — until {consensusPct}% of its analyses
            agree on its best move, or until it has been analysed {maxResults} times, when it is
            settled without a consensus. The job is done once every rack is settled.
          {/if}
        </p>
      {/if}
    </fieldset>
    <p class="text-xs text-muted-foreground">
      The recorder is shown because it decides whether a static player can rank anything at
      all: <strong>best</strong> keeps only the top move, so every rack would come back with one
      analysis however many plays the config says to record. A simulating player ranks every
      play up to its number of plays, whatever its recorder. Tasks address <em>ranges</em> of the
      rack space and are generated as workers claim them, so creating the job writes no rows
      however large the space is.
    </p>
  {:else if jobType === 'games' || jobType === 'game_pairs'}
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="label" for="p1">Player 1</label>
        <select id="p1" class="input" bind:value={player1} required>
          {#each configs as config}<option value={config.id}>{config.name}</option>{/each}
        </select>
      </div>
      <div>
        <label class="label" for="p2">Player 2</label>
        <select id="p2" class="input" bind:value={player2} required>
          {#each configs as config}<option value={config.id}>{config.name}</option>{/each}
        </select>
      </div>
    </div>
    <div class="grid {testEnabled ? 'grid-cols-3' : 'grid-cols-2'} gap-3">
      <div>
        <label class="label" for="batch">
          {units} Per Task
        </label>
        <input
          id="batch"
          type="number"
          min={jobType === 'games' ? 2 : 1}
          max={(jobType === 'games' ? 10000 : 5000) / (capturePositions ? 10 : 1)}
          step={jobType === 'games' ? 2 : 1}
          class="input"
          bind:value={batchSize}
        />
        {#if jobType === 'games'}
          <p class="mt-1 text-xs text-muted-foreground">
            Even: MAGPIE gives player 1 the first move in a task's first game and alternates, so
            an odd batch hands player 1 the first move more often.
          </p>
        {/if}
      </div>
      {#if testEnabled}
        <div>
          <label class="label" for="min">Minimum {units}</label>
          <input id="min" type="number" min="1" max={maxUnits} class="input" bind:value={minUnits} />
        </div>
      {/if}
      <div>
        <label class="label" for="max">
          {testEnabled ? `Maximum ${units}` : `${units} To Play`}
        </label>
        <input id="max" type="number" min="1" class="input" bind:value={maxUnits} />
      </div>
    </div>
    <div>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={testEnabled} />
        <span class="label mb-0">Significance Test</span>
      </label>
      <p class="mt-1 text-xs text-muted-foreground">
        {#if testEnabled}
          The job stops as soon as one player is shown to be better at the confidence below (once
          the minimum is played), or at its maximum if neither is. The test keeps an interval
          around player 1's score that stays valid however often it is checked.
        {:else}
          Without a test the job plays every {jobType === 'games' ? 'game' : 'pair'} it is set
          to, and its result is the match score.
        {/if}
      </p>
    </div>
    {#if testEnabled}
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="label" for="confidence">Confidence %</label>
          <input
            id="confidence"
            type="number"
            step="any"
            min="50.1"
            max="99.99"
            class="input"
            bind:value={confidencePct}
          />
          <p class="mt-1 text-xs text-muted-foreground">
            The chance of naming a winner between two equal players is at most about
            {Math.round((100 - confidencePct) * 100) / 100}%. Higher takes more {units.toLowerCase()}
            to decide.
          </p>
        </div>
      </div>
    {/if}
    <div>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={capturePositions} />
        <span class="label mb-0">Position Recorder</span>
      </label>
      <p class="mt-1 text-xs text-muted-foreground">
        Keeps the position analysed on every turn of every game, with its ranked moves, for
        signed-in users to search on the job's page. It roughly doubles the rows a job produces,
        and a batch is at most {jobType === 'games' ? '1,000 games' : '500 pairs'} while saving.
        A static player ranks its plays on every turn, up to the number it records, which slows
        its games; a simming player's ranking costs nothing extra.
      </p>
      {#if jobType === 'game_pairs' && capturePositions}
        <label class="mt-2 flex items-center gap-2">
          <input type="checkbox" bind:checked={captureFirstDivergence} />
          <span class="label mb-0">Only Where Each Pair First Diverges</span>
        </label>
        <p class="mt-1 text-xs text-muted-foreground">
          A pair's two games are one game with the seats swapped until the players choose
          different moves. Keeps just that turn: the position, once from each game, with each
          player's ranking. A pair played identically keeps nothing.
        </p>
      {/if}
    </div>
  {:else}
    {#if leaveConflict}
      <p class="field-error">{leaveConflict} Pick a static config that sorts on equity.</p>
    {/if}
    <p class="text-xs text-muted-foreground">
      The bot plays both seats as this player, with its lexicon and its wordmap setting. Its
      leaves are not used: generation 1 plays with a zeroed KLV the server builds, and every later
      generation plays with the KLV built from the one before.
    </p>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="label" for="iters">Games Per Task</label>
        <input id="iters" type="number" min="1" class="input" bind:value={numIterations} />
      </div>
      <div>
        <label class="label" for="rpt">Racks Per Task</label>
        <input id="rpt" type="number" min="1" max="10000" class="input" bind:value={racksPerTask} />
      </div>
    </div>
    <div>
      <label class="label" for="targets">Target Per Rack, Per Generation</label>
      <input
        id="targets"
        class="input"
        inputmode="numeric"
        placeholder="100, 200, 500, 1000"
        bind:value={targetRackCounts}
      />
      <p class="mt-1 text-xs text-muted-foreground">
        {#if 'error' in targets}
          <span class="field-error">{targets.error}</span>
        {:else}
          {targets.targets.length}
          {targets.targets.length === 1 ? 'generation' : 'generations'}:
          <span class="tabular-nums text-foreground">{targetsText(targets.targets)}</span>. Each
          closes once every rack has occurred its own target number of times, as in MAGPIE's
          <code>leavegen 100,200,500,…</code>. Separate targets with commas or spaces, and write
          no thousands separators.
        {/if}
      </p>
    </div>
  {/if}

  <!-- Announced: an error that appears after a submit is otherwise silent to a
       screen reader. -->
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create job'}</button>
</form>
