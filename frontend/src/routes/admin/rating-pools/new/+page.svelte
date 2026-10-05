<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api, errorText, type InputData, type PlayerConfig } from '$lib/api';
  import { blankFields, unchosenText } from '$lib/format';

  let configs: PlayerConfig[] = [];
  let files: InputData[] = [];
  let error = '';
  // Whether `error` is the last submit's: only that is cleared by an edit, not
  // a failure to load the form's choices.
  let fromSubmit = false;
  let busy = false;
  // Set once the pool exists, so a failure adding its members is not answered
  // by a retry that makes a second pool.
  let createdId = '';

  let name = '';
  let variant = 'classic';
  let letterdistId = '';
  let layoutId = '';
  let anchorId = '';
  let anchorRating = 2000;
  // Everyone else to rate, added once the pool exists. Each addition refits
  // the pool, so this is a convenience over adding them one at a time on the
  // pool's page, not a different operation.
  let members: string[] = [];

  $: letterdists = files.filter((f) => f.role === 'letterdist');
  $: layouts = files.filter((f) => f.role === 'layout');
  $: others = configs.filter((config) => config.id !== anchorId);

  function label(file: InputData): string {
    return `${file.name} (${file.tarball_date}, ${file.sha256.slice(0, 8)})`;
  }

  onMount(async () => {
    try {
      [configs, files] = await Promise.all([api.playerConfigs(), api.inputData()]);
      // The letter distribution and board are left for the admin to choose,
      // as on the job form: the first of each was only whichever was
      // imported first.
      anchorId = configs[0]?.id ?? '';
    } catch (e) {
      error = `Could not load player configs and input data: ${e instanceof Error ? e.message : String(e)}`;
    }
  });

  async function submit() {
    error = '';
    // As on the job form: the browser holds an empty required select, and
    // this names one that gets past it.
    const unchosen = unchosenText({
      'a letter distribution': letterdistId,
      'a board layout': layoutId
    });
    if (unchosen) {
      error = unchosen;
      fromSubmit = true;
      return;
    }
    const request = {
      name: name.trim(),
      variant,
      letterdist_id: letterdistId,
      layout_id: layoutId,
      anchor_player_config_id: anchorId,
      anchor_rating: anchorRating
    };
    // A cleared number box binds as null.
    const blank = blankFields(request);
    if (blank.length) {
      error = `Fill in every setting: ${blank.join(', ')} is empty.`;
      fromSubmit = true;
      return;
    }
    busy = true;
    try {
      createdId = (await api.createRatingPool(request)).id;
      for (const id of members.filter((member) => member !== anchorId)) {
        await api.addRatingPoolMember(createdId, id);
      }
      goto(`/ratings/${createdId}`);
    } catch (e) {
      error = createdId
        ? `The pool was created, but adding its members stopped: ${errorText(e)}. Add the rest from the pool's page.`
        : errorText(e);
      fromSubmit = !createdId;
    } finally {
      busy = false;
    }
  }
</script>

<h1 class="mb-2 text-2xl font-semibold">Create a rating pool</h1>
<p class="mb-6 max-w-2xl text-sm text-muted-foreground">
  A pool rates its members jointly from the game pairs every game-pairs job between two of them
  has played under its variant, letter distribution and board. The anchor is pinned at its rating and every
  other rating is relative to it, so ratings compare within a pool and not between pools.
</p>

<!-- Any edit clears the last server error: a submit the browser blocks never
     reaches submit(), which is where it was cleared, so it lingered. -->
<form class="card max-w-2xl space-y-4" on:submit|preventDefault={submit} on:input={() => { if (fromSubmit) { error = ''; fromSubmit = false; } }}>
  <div>
    <label class="label" for="name">Pool name</label>
    <input id="name" class="input" bind:value={name} required placeholder="e.g. CSW24 static and 1-ply" />
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
    Only jobs created with these three settings count as evidence.
  </p>

  <div class="grid grid-cols-2 gap-3">
    <div>
      <label class="label" for="anchor">Anchor player config</label>
      <select id="anchor" class="input" bind:value={anchorId} required>
        {#each configs as config}<option value={config.id}>{config.name}</option>{/each}
      </select>
    </div>
    <div>
      <label class="label" for="anchor-rating">Anchor rating</label>
      <input id="anchor-rating" type="number" step="any" min="-10000" max="10000" class="input" bind:value={anchorRating} />
    </div>
  </div>

  {#if others.length}
    <fieldset>
      <legend class="label">Other members</legend>
      <div class="grid gap-1 sm:grid-cols-2">
        {#each others as config}
          <label class="flex items-center gap-2 text-sm">
            <input type="checkbox" value={config.id} bind:group={members} />
            {config.name}
          </label>
        {/each}
      </div>
      <p class="mt-1 text-xs text-muted-foreground">
        Optional: members can also be added and removed on the pool's page. A member with no
        game pairs against another member has no rating yet.
      </p>
    </fieldset>
  {/if}

  <!-- Announced: an error that appears after a submit is otherwise silent to a
       screen reader. -->
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
  {#if createdId}
    <a class="btn-primary no-underline hover:no-underline" href="/ratings/{createdId}">Open the pool</a>
  {:else}
    <button class="btn-primary" disabled={busy}>{busy ? 'Creating…' : 'Create rating pool'}</button>
  {/if}
</form>
