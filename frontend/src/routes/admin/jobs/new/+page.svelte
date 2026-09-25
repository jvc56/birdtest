<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api, errorText, type InputData, type JobType, type PlayerConfig } from '$lib/api';
  import { blankFields, jobTypeLabel } from '$lib/format';

  let configs: PlayerConfig[] = [];
  let files: InputData[] = [];
  let error = '';
  let busy = false;

  let jobType: JobType = 'game_pairs';
  let redundancy = 1;
  // Pre-filled from the server-wide floor rather than left blank: a default
  // nobody sees is how every new job quietly inherits a floor that is too low.
  let minMagpieVersion = '';
  let serverFloor = '';
  let variant = 'classic';
  let letterdistId = '';
  let layoutId = '';
  let leaveKwgId = '';

  $: letterdists = files.filter((f) => f.role === 'letterdist');
  $: layouts = files.filter((f) => f.role === 'layout');
  $: lexica = files.filter((f) => f.role === 'kwg');

  function label(file: InputData): string {
    return `${file.name} (${file.tarball_date}, ${file.sha256.slice(0, 8)})`;
  }

  // Per-type fields. Only the ones the selected type uses are submitted.
  let playerConfigId = '';
  let player1 = '';
  let player2 = '';
  let batchSize = 1;
  let minUnits = 1000;
  let maxUnits = 40000;
  let sprtAlpha = 0.05;
  let sprtBeta = 0.05;
  let eloLow = -10;
  let eloHigh = 10;
  let numIterations = 10000;
  let generationCount = 1;
  let targetRackCount = 500;
  let racksPerTask = 50;
  let leaveUseWordmap = true;

  const types: JobType[] = ['opening_rack', 'games', 'game_pairs', 'leave_generation'];

  // The combination job creation refuses, surfaced before the submit rather
  // than as the error that comes back from it: `-r best` is MOVE_RECORD_BEST,
  // so movegen keeps one play and the rest of the ranking never exists.
  $: selectedConfig = configs.find((config) => config.id === playerConfigId);
  $: openingRackConflict =
    jobType === 'opening_rack' &&
    selectedConfig &&
    selectedConfig.recorder_type === 'best' &&
    selectedConfig.num_plays_recorded > 1
      ? `${selectedConfig.name} records only the best move, so this job would store one play per rack rather than the ${selectedConfig.num_plays_recorded} it asks for.`
      : null;

  function firstOfRole(role: string): string {
    return files.find((f) => f.role === role)?.id ?? '';
  }

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
    // Filtered from `files` here rather than read off the `$:` arrays above:
    // those are recomputed on the update cycle, not on assignment, so they
    // would still be empty on the next line and every default would be ''.
    letterdistId = firstOfRole('letterdist');
    layoutId = firstOfRole('layout');
    leaveKwgId = firstOfRole('kwg');
    serverFloor = (await api.clientVersion()).min_magpie_version;
    minMagpieVersion = serverFloor;
  }

  function body(): Record<string, unknown> {
    const common = {
      job_type: jobType,
      // Leave generation runs at redundancy 1 only: the server refuses more.
      redundancy: jobType === 'leave_generation' ? 1 : redundancy,
      variant,
      letterdist_id: letterdistId,
      layout_id: layoutId,
      ...(minMagpieVersion ? { min_magpie_version: minMagpieVersion } : {})
    };
    const sprt = {
      sprt_alpha: sprtAlpha,
      sprt_beta: sprtBeta,
      elo_low: eloLow,
      elo_high: eloHigh
    };
    switch (jobType) {
      case 'opening_rack':
        return { ...common, player_config_id: playerConfigId };
      case 'games':
        return {
          ...common,
          player1_config_id: player1, player2_config_id: player2,
          games_per_batch: batchSize, min_games: minUnits, max_games: maxUnits, ...sprt
        };
      case 'game_pairs':
        return {
          ...common,
          player1_config_id: player1, player2_config_id: player2,
          pairs_per_batch: batchSize, min_pairs: minUnits, max_pairs: maxUnits, ...sprt
        };
      case 'leave_generation':
        return {
          ...common,
          kwg_id: leaveKwgId,
          num_iterations: numIterations,
          generation_count: generationCount,
          target_rack_count: targetRackCount,
          racks_per_task: racksPerTask,
          use_wordmap: leaveUseWordmap
        };
    }
  }

  async function submit() {
    error = '';
    // A cleared number box binds as null, and the server's answer to a null
    // setting names the whole request ("data did not match any variant"),
    // not the field.
    const request = body();
    const blank = blankFields(request);
    if (blank.length) {
      error = `Fill in every setting: ${blank.join(', ')} is empty.`;
      return;
    }
    busy = true;
    try {
      const created = await api.createJob(request);
      goto(`/admin/jobs/${created.job.id}`);
    } catch (e) {
      error = errorText(e);
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

<form class="card max-w-2xl space-y-4" on:submit|preventDefault={submit}>
  <div>
    <label class="label" for="type">Job type</label>
    <select id="type" class="input" bind:value={jobType}>
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
      <input id="variant" class="input" bind:value={variant} />
    </div>
    <div>
      <label class="label" for="ld">Letter distribution</label>
      <select id="ld" class="input" bind:value={letterdistId} required>
        {#each letterdists as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
    <div>
      <label class="label" for="layout">Board layout</label>
      <select id="layout" class="input" bind:value={layoutId} required>
        {#each layouts as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
    </div>
  </div>
  <p class="text-xs text-muted-foreground">
    One distribution and one board per job: MAGPIE takes a single bag and board for the whole game.
    Each player's lexicon and leaves come from its own config.
  </p>

  {#if jobType === 'opening_rack'}
    <div>
      <label class="label" for="pc">Player config</label>
      <select id="pc" class="input" bind:value={playerConfigId}>
        {#each configs as config}
          <option value={config.id}>
            {config.name} — recorder {config.recorder_type}, {config.num_plays_recorded} play{config.num_plays_recorded === 1
              ? ''
              : 's'} recorded
          </option>
        {/each}
      </select>
    </div>
    {#if openingRackConflict}
      <p class="field-error">
        {openingRackConflict} Pick a config whose recorder is <strong>all</strong> or
        <strong>equity</strong>, or one that records a single play.
      </p>
    {/if}
    <p class="text-xs text-muted-foreground">
      The recorder is shown because it decides whether this job can rank anything at all:
      <strong>best</strong> keeps only the top move, so every rack would come back with one
      analysis however many plays the config says to record. Tasks address <em>ranges</em> of the
      rack space and are generated as workers claim them, so creating the job writes no rows
      however large the space is.
    </p>
  {:else if jobType === 'games' || jobType === 'game_pairs'}
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="label" for="p1">Player 1</label>
        <select id="p1" class="input" bind:value={player1}>
          {#each configs as config}<option value={config.id}>{config.name}</option>{/each}
        </select>
      </div>
      <div>
        <label class="label" for="p2">Player 2</label>
        <select id="p2" class="input" bind:value={player2}>
          {#each configs as config}<option value={config.id}>{config.name}</option>{/each}
        </select>
      </div>
    </div>
    <div class="grid grid-cols-3 gap-3">
      <div>
        <label class="label" for="batch">
          {jobType === 'games' ? 'Games' : 'Pairs'} per batch
        </label>
        <input id="batch" type="number" min="1" class="input" bind:value={batchSize} />
      </div>
      <div>
        <label class="label" for="min">Min before SPRT</label>
        <input id="min" type="number" min="1" class="input" bind:value={minUnits} />
      </div>
      <div>
        <label class="label" for="max">Hard cap</label>
        <input id="max" type="number" min="1" class="input" bind:value={maxUnits} />
      </div>
    </div>
    <div class="grid grid-cols-4 gap-3">
      <div><label class="label" for="alpha">α</label><input id="alpha" type="number" step="0.01" class="input" bind:value={sprtAlpha} /></div>
      <div><label class="label" for="beta">β</label><input id="beta" type="number" step="0.01" class="input" bind:value={sprtBeta} /></div>
      <div><label class="label" for="lo">Elo low (H0)</label><input id="lo" type="number" class="input" bind:value={eloLow} /></div>
      <div><label class="label" for="hi">Elo high (H1)</label><input id="hi" type="number" class="input" bind:value={eloHigh} /></div>
    </div>
  {:else}
    <div>
      <label class="label" for="leavekwg">Lexicon</label>
      <select id="leavekwg" class="input" bind:value={leaveKwgId} required>
        {#each lexica as file}<option value={file.id}>{label(file)}</option>{/each}
      </select>
      <p class="mt-1 text-xs text-muted-foreground">
        Leave generation has one bot and no player config, so its lexicon sits on the job. It needs
        no leaves file: generation 1 plays with a zeroed KLV the server builds, and every later
        generation plays with the KLV built from the one before.
      </p>
    </div>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="label" for="iters">Games per task</label>
        <input id="iters" type="number" min="1" class="input" bind:value={numIterations} />
      </div>
      <div>
        <label class="label" for="gens">Generations</label>
        <input id="gens" type="number" min="1" class="input" bind:value={generationCount} />
      </div>
    </div>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="label" for="target">Occurrences per rack</label>
        <input id="target" type="number" min="1" class="input" bind:value={targetRackCount} />
      </div>
      <div>
        <label class="label" for="rpt">Racks per task</label>
        <input id="rpt" type="number" min="1" class="input" bind:value={racksPerTask} />
      </div>
    </div>
    <label class="flex items-center gap-2">
      <input type="checkbox" bind:checked={leaveUseWordmap} />
      <span class="label mb-0">Use wordmap</span>
    </label>
  {/if}

  {#if error}<p class="field-error">{error}</p>{/if}
  <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create job'}</button>
</form>
