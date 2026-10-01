<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api, errorText, type InputData, type JobType, type PlayerConfig } from '$lib/api';
  import { blankFields, jobTypeLabel, leavePlayerConflict, parseTargetRackCounts, unchosenText } from '$lib/format';

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
  let redundancy = 1;
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
  let sprtEnabled = false;
  let minUnits = 1000;
  let maxUnits = 40000;
  let sprtAlpha = 0.05;
  let sprtBeta = 0.05;
  let eloLow = -10;
  let eloHigh = 10;
  // Keep every position the games analyse, searchable on the job's page.
  let capturePositions = false;
  let numIterations = 10000;
  // One occurrence target per generation, as MAGPIE's `leavegen` takes them:
  // the list's length is how many generations the job runs.
  let targetRackCounts = '500';
  $: targets = parseTargetRackCounts(targetRackCounts);
  let racksPerTask = 50;

  const types: JobType[] = ['opening_rack', 'games', 'game_pairs', 'leave_generation'];

  // The two combinations job creation refuses, surfaced before the submit
  // rather than as the error that comes back from it. `-r best` is
  // MOVE_RECORD_BEST, so a static player's movegen keeps one play and the rest
  // of the ranking never exists (a simmer ranks every play up to num_plays
  // whatever its recorder). And no player reports more plays than num_plays,
  // which sizes the move list.
  $: selectedConfig = configs.find((config) => config.id === playerConfigId);
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
      // Leave generation runs at redundancy 1 only: the server refuses more.
      redundancy: jobType === 'leave_generation' ? 1 : redundancy,
      variant,
      letterdist_id: letterdistId,
      layout_id: layoutId,
      bingo_bonus: bingoBonus,
      // A leave job's bot never simulates, so it states no cutoff: the server
      // refuses one.
      ...(jobType === 'leave_generation' ? {} : { sim_cutoff: simCutoff }),
      ...(minMagpieVersion ? { min_magpie_version: minMagpieVersion } : {})
    };
    const sprt = sprtEnabled
      ? {
          sprt_enabled: true,
          sprt_alpha: sprtAlpha,
          sprt_beta: sprtBeta,
          elo_low: eloLow,
          elo_high: eloHigh
        }
      : { sprt_enabled: false };
    switch (jobType) {
      case 'opening_rack':
        return { ...common, player_config_id: playerConfigId };
      case 'games':
        return {
          ...common,
          player1_config_id: player1, player2_config_id: player2,
          games_per_batch: batchSize, max_games: maxUnits, ...sprt,
          ...(sprtEnabled ? { min_games: minUnits } : {}),
          capture_positions: capturePositions
        };
      case 'game_pairs':
        return {
          ...common,
          player1_config_id: player1, player2_config_id: player2,
          pairs_per_batch: batchSize, max_pairs: maxUnits, ...sprt,
          ...(sprtEnabled ? { min_pairs: minUnits } : {}),
          capture_positions: capturePositions
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
    <label class="label" for="name">Job name</label>
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
    <label class="label" for="type">Job type</label>
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
      <label class="label" for="redundancy">Redundancy</label>
      <input id="redundancy" type="number" min="1" class="input" bind:value={redundancy} disabled={jobType === 'leave_generation'} />
    </div>
    <div>
      <label class="label" for="magpie">Min MAGPIE version</label>
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
      <label class="label" for="ld">Letter distribution</label>
      <select id="ld" class="input" bind:value={letterdistId} required>
        <option value="" disabled selected>Choose…</option>
        {#each letterdists as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
    <div>
      <label class="label" for="layout">Board layout</label>
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
      <label class="label" for="bingo">Bingo bonus (-bb)</label>
      <input id="bingo" type="number" min="0" step="1" required class="input" bind:value={bingoBonus} />
    </div>
    {#if jobType !== 'leave_generation'}
      <div>
        <label class="label" for="cutoff">Sim cutoff (-cutoff)</label>
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
      <label class="label" for="pc">Player config</label>
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
    <div class="grid {sprtEnabled ? 'grid-cols-3' : 'grid-cols-2'} gap-3">
      <div>
        <label class="label" for="batch">
          {jobType === 'games' ? 'Games' : 'Pairs'} per batch
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
      {#if sprtEnabled}
        <div>
          <label class="label" for="min">Min before SPRT</label>
          <input id="min" type="number" min="0" class="input" bind:value={minUnits} />
        </div>
      {/if}
      <div>
        <label class="label" for="max">
          {sprtEnabled ? 'Hard cap' : jobType === 'games' ? 'Games to play' : 'Pairs to play'}
        </label>
        <input id="max" type="number" min="1" class="input" bind:value={maxUnits} />
      </div>
    </div>
    <div>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={sprtEnabled} />
        <span class="label mb-0">Run an SPRT</span>
      </label>
      <p class="mt-1 text-xs text-muted-foreground">
        {#if sprtEnabled}
          The job stops as soon as the test decides between the two Elo hypotheses (once the
          minimum is played), or at the hard cap if it never does.
        {:else}
          Without a test the job plays every {jobType === 'games' ? 'game' : 'pair'} it is set
          to, and its result is the match score.
        {/if}
      </p>
    </div>
    {#if sprtEnabled}
      <div class="grid grid-cols-4 gap-3">
        <div><label class="label" for="alpha">α</label><input id="alpha" type="number" step="any" min="0.000001" max="0.999999" class="input" bind:value={sprtAlpha} /></div>
        <div><label class="label" for="beta">β</label><input id="beta" type="number" step="any" min="0.000001" max="0.999999" class="input" bind:value={sprtBeta} /></div>
        <div><label class="label" for="lo">Elo low (H0)</label><input id="lo" type="number" step="any" min="-1000" max="1000" class="input" bind:value={eloLow} /></div>
        <div><label class="label" for="hi">Elo high (H1)</label><input id="hi" type="number" step="any" min="-1000" max="1000" class="input" bind:value={eloHigh} /></div>
      </div>
    {/if}
    <div>
      <label class="flex items-center gap-2">
        <input type="checkbox" bind:checked={capturePositions} />
        <span class="label mb-0">Save the positions played</span>
      </label>
      <p class="mt-1 text-xs text-muted-foreground">
        Keeps the position analysed on every turn of every game, with its ranked moves, for
        signed-in users to search on the job's page. It roughly doubles the rows a job produces,
        and a batch is at most {jobType === 'games' ? '1,000 games' : '500 pairs'} while saving.
        A static player records only the move it played; a simming player, its whole ranking.
      </p>
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
        <label class="label" for="iters">Games per task</label>
        <input id="iters" type="number" min="1" class="input" bind:value={numIterations} />
      </div>
      <div>
        <label class="label" for="rpt">Racks per task</label>
        <input id="rpt" type="number" min="1" max="10000" class="input" bind:value={racksPerTask} />
      </div>
    </div>
    <div>
      <label class="label" for="targets">Occurrences per rack, per generation</label>
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
          {targets.targets.length === 1 ? 'generation' : 'generations'}. Each closes once every
          rack has occurred its own target number of times, as in MAGPIE's
          <code>leavegen 100,200,500,…</code>.
        {/if}
      </p>
    </div>
  {/if}

  <!-- Announced: an error that appears after a submit is otherwise silent to a
       screen reader. -->
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create job'}</button>
</form>
