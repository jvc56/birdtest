<script lang="ts">
  import { goto } from '$app/navigation';
  import { api } from '$lib/api';

  let name = '';
  let recorderType = 'best';
  let sortStrategy: string = 'equity';
  let leaves = '';
  let simming = false;
  let maxIterations = 1000;
  let plies = 2;
  let topPlays = 10;
  let stoppingPct = 99;
  let useInference = false;
  let timeLimitSecs: number | '' = '';
  let error = '';
  let busy = false;

  async function submit() {
    busy = true;
    error = '';
    try {
      const created = await api.createPlayerConfig({
        name,
        recorder_type: recorderType,
        // A simming player's move comes from the simulation, not a static sort.
        sort_strategy: simming ? null : sortStrategy,
        leaves: leaves || null,
        max_iterations: simming ? maxIterations : null,
        plies: simming ? plies : null,
        top_plays: simming ? topPlays : null,
        stopping_pct: simming ? stoppingPct : null,
        use_inference: simming ? useInference : null,
        time_limit_secs: simming && timeLimitSecs !== '' ? Number(timeLimitSecs) : null
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
        <option value="best">best — play the top-ranked move (right for autoplay)</option>
        <option value="equity">equity — record all moves within the equity margin</option>
        <option value="all">all — record every move</option>
      </select>
    </div>
    <div>
      <label class="label" for="leaves">Leaves file (-k)</label>
      <input id="leaves" class="input" bind:value={leaves} placeholder="lexicon default" />
    </div>
  </div>

  <label class="flex items-center gap-2 text-sm">
    <input type="checkbox" bind:checked={simming} />
    Simming player
  </label>

  {#if simming}
    <div class="grid grid-cols-2 gap-3">
      <div><label class="label" for="iters">Max iterations (-i)</label><input id="iters" type="number" class="input" bind:value={maxIterations} /></div>
      <div><label class="label" for="plies">Plies (-pl)</label><input id="plies" type="number" class="input" bind:value={plies} /></div>
      <div><label class="label" for="np">Top plays (-np)</label><input id="np" type="number" class="input" bind:value={topPlays} /></div>
      <div><label class="label" for="sc">Stopping % (-sc)</label><input id="sc" type="number" step="0.1" class="input" bind:value={stoppingPct} /></div>
      <div><label class="label" for="tl">Time limit seconds (-tl)</label><input id="tl" type="number" step="0.1" class="input" bind:value={timeLimitSecs} /></div>
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

  {#if error}<p class="field-error">{error}</p>{/if}
  <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create'}</button>
</form>
