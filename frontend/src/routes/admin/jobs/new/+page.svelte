<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api, type JobType, type PlayerConfig } from '$lib/api';
  import { jobTypeLabel } from '$lib/format';

  let configs: PlayerConfig[] = [];
  let error = '';
  let busy = false;

  let jobType: JobType = 'game_pairs';
  let priority = 0;
  let redundancy = 1;
  let minMagpieVersion = '';
  let lexicon = 'NWL23';
  let variant = 'classic';
  let letterDistribution = 'english';

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
  let maxLeaveSize = 6;
  let leaveUseWordmap = true;

  const types: JobType[] = ['opening_rack', 'games', 'game_pairs', 'leave_generation'];

  onMount(async () => {
    configs = await api.playerConfigs();
    if (configs.length) {
      playerConfigId = configs[0].id;
      player1 = configs[0].id;
      player2 = configs[configs.length - 1].id;
    }
  });

  function body(): Record<string, unknown> {
    const common = {
      job_type: jobType,
      priority,
      redundancy,
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
        return { ...common, lexicon, variant, letter_distribution: letterDistribution, player_config_id: playerConfigId };
      case 'games':
        return {
          ...common, lexicon, variant, letter_distribution: letterDistribution,
          player1_config_id: player1, player2_config_id: player2,
          games_per_batch: batchSize, min_games: minUnits, max_games: maxUnits, ...sprt
        };
      case 'game_pairs':
        return {
          ...common, lexicon, variant, letter_distribution: letterDistribution,
          player1_config_id: player1, player2_config_id: player2,
          pairs_per_batch: batchSize, min_pairs: minUnits, max_pairs: maxUnits, ...sprt
        };
      case 'leave_generation':
        return {
          ...common, lexicon, variant, letter_distribution: letterDistribution,
          num_iterations: numIterations,
          generation_count: generationCount,
          target_rack_count: targetRackCount,
          racks_per_task: racksPerTask,
          max_leave_size: maxLeaveSize,
          use_wordmap: leaveUseWordmap
        };
    }
  }

  async function submit() {
    busy = true;
    error = '';
    try {
      const created = await api.createJob(body());
      goto(`/admin/jobs/${created.job.id}`);
    } catch (e) {
      error = (e as Error).message;
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

  <div class="grid grid-cols-3 gap-3">
    <div>
      <label class="label" for="priority">Priority (lower wins)</label>
      <input id="priority" type="number" class="input" bind:value={priority} />
    </div>
    <div>
      <label class="label" for="redundancy">Redundancy</label>
      <input id="redundancy" type="number" min="1" class="input" bind:value={redundancy} />
    </div>
    <div>
      <label class="label" for="magpie">Min MAGPIE version</label>
      <input id="magpie" class="input" bind:value={minMagpieVersion} placeholder="1.4.0" />
    </div>
  </div>

  <div class="grid grid-cols-2 gap-3">
    <div>
      <label class="label" for="lexicon">Lexicon</label>
      <input id="lexicon" class="input" bind:value={lexicon} />
    </div>
    <div>
      <label class="label" for="variant">Variant</label>
      <input id="variant" class="input" bind:value={variant} />
    </div>
    <div>
      <label class="label" for="ld">Letter distribution</label>
      <input id="ld" class="input" bind:value={letterDistribution} />
    </div>
  </div>

  {#if jobType === 'opening_rack'}
    <div>
      <label class="label" for="pc">Player config</label>
      <select id="pc" class="input" bind:value={playerConfigId}>
        {#each configs as config}<option value={config.id}>{config.name}</option>{/each}
      </select>
    </div>
    <p class="text-xs text-muted-foreground">
      Every distinct 7-tile rack drawable from this lexicon's bag becomes one task at creation
      time. For a full English bag that is a large number of rows — use a small distribution while
      testing.
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
    <div class="grid grid-cols-3 gap-3">
      <div>
        <label class="label" for="target">Occurrences per rack</label>
        <input id="target" type="number" min="1" class="input" bind:value={targetRackCount} />
      </div>
      <div>
        <label class="label" for="rpt">Racks per task</label>
        <input id="rpt" type="number" min="1" class="input" bind:value={racksPerTask} />
      </div>
      <div>
        <label class="label" for="mls">Max leave size</label>
        <input id="mls" type="number" min="1" max="6" class="input" bind:value={maxLeaveSize} />
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
