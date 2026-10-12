<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type JobListItem } from '$lib/api';
  import { jobTitle, jobTypeLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';

  let active: JobListItem[] = [];

  // The server `magpie contribute` uses when contribute.txt sets none
  // (MAGPIE's CONTRIBUTE_DEFAULT_SERVER). Any other server -- a local or test
  // stack -- has to be named in the file.
  const DEFAULT_SERVER = 'https://birdtest.org';
  const origin = typeof window !== 'undefined' ? window.location.origin : DEFAULT_SERVER;

  // contribute.txt's settings, for a table on a wide screen and a stacked
  // list on a phone: as a table, the descriptions were a column squeezed to a
  // word a line and then cut off at the screen's edge.
  const SETTINGS: { name: string; default: string; mono?: true; what: string }[] = [
    { name: 'server', default: DEFAULT_SERVER, mono: true, what: 'The birdtest site to contribute to.' },
    {
      name: 'apikey',
      default: 'none',
      what: 'An API key from your account (step 2), to contribute under your username. Without one you contribute anonymously.'
    },
    { name: 'threads', default: 'every core but one', what: 'How many threads to run tasks on.' },
    { name: 'maxtasks', default: '0', what: 'How many tasks to run before stopping. 0 runs until you stop it.' },
    { name: 'idlewait', default: '5', what: 'Seconds to wait before asking again when there is no work.' }
  ];

  // An example contribute.txt naming every setting, pointed at the site it is
  // read on: a local or test stack's own address, not the default server's.
  const EXAMPLE_VALUES: Record<string, string> = {
    server: origin,
    apikey: '<your API key from step 2>',
    threads: '4',
    maxtasks: '0',
    idlewait: '5'
  };
  const EXAMPLE = SETTINGS.map((s) => `${s.name.padEnd(9)}${EXAMPLE_VALUES[s.name]}`).join('\n');

  // The job list is a side panel here; a failed load leaves it empty rather
  // than the whole page broken, and is not an unhandled rejection.
  onMount(async () => {
    try {
      // Filtered by the server: filtered here, only the newest page's
      // active jobs were ever shown.
      active = (await api.jobs(0, 'active')).items;
    } catch {
      active = [];
    }
  });
</script>

<section class="space-y-8">
  <div class="space-y-3">
    <h1 class="text-3xl font-semibold">Crowdsourced Crossword Game Research</h1>
    <div class="max-w-2xl space-y-2 text-muted-foreground" data-testid="intro">
      <p>
        birdtest is a distributed task queue for researching crossword games which support the following jobs:
      </p>
      <ul class="list-disc space-y-1 pl-5">
        <li>
          <span class="font-medium text-foreground">Games</span>: two players play a series of
          games against each other, and their wins, average scores and spread are compared. An
          optional significance test can decide whether one player is better.
        </li>
        <li>
          <span class="font-medium text-foreground">Game Pairs</span>: like Games, but every game
          is played twice with the same tiles and the players' seats swapped, which cancels out
          much of the luck of the draw, so a difference between the players shows up sooner.
        </li>
        <li>
          <span class="font-medium text-foreground">Opening Rack Analysis</span>: every single opening rack for a given distribution is analyzed by a single player and the results are recorded to use as an opening book.
        </li>
        <li>
          <span class="font-medium text-foreground">Leave Generation</span>: many games are played
          to measure what each leave (the tiles kept after a play) is worth, generation after
          generation, producing the leave values MAGPIE uses to judge its moves.
        </li>
      </ul>
      <p>
         Admins create jobs while workers running MAGPIE claim tasks to work on and submit the results to birdtest.
      </p>
    </div>
    <div class="flex gap-3">
      <a href="/jobs" class="btn-primary no-underline hover:no-underline">Browse jobs</a>
      <a href="/register" class="btn-secondary no-underline hover:no-underline">Create an account</a>
    </div>
  </div>

  {#if active.length}
    <div class="space-y-3">
      <h2 class="text-lg font-medium">Active Jobs</h2>
      <div class="grid gap-3 sm:grid-cols-2">
        {#each active as job}
          <a href="/jobs/{job.id}" class="card no-underline hover:border-primary/50 hover:no-underline">
            <div class="flex items-center justify-between">
              <span class="font-medium text-foreground">{jobTitle(job)}</span>
              <JobStatusBadge status={job.status} />
            </div>
            <p class="mt-1 text-sm text-muted-foreground">
              {#if job.name}{jobTypeLabel(job.job_type)} · {/if}{job.allocation}% allocation
            </p>
          </a>
        {/each}
      </div>
    </div>
  {/if}

  <div class="space-y-3" data-testid="contribute">
    <h2 class="text-lg font-medium">Contribute</h2>
    <ol class="list-decimal space-y-5 pl-5 text-sm marker:font-medium">
      <li class="space-y-2">
        <h3 class="font-medium">Install MAGPIE</h3>
        <p class="text-muted-foreground">
          Follow
          <a href="https://github.com/jvc56/MAGPIE#getting-started">these instructions</a> to
          install MAGPIE on your computer.
        </p>
      </li>
      <li class="space-y-2">
        <h3 class="font-medium">(Optional) Create an account</h3>
        <p class="text-muted-foreground">
          To contribute under a username, <a href="/register">create an account</a>, make an API
          key on your account page, and copy it: you'll put it in contribute.txt in the next step.
          Otherwise you can contribute anonymously.
        </p>
      </li>
      <li class="space-y-2">
        <h3 class="font-medium">(Optional) Create a contribute.txt file</h3>
        <p class="text-muted-foreground">
          Workers can specify certain settings. Any setting not specified in the contribute.txt
          file uses the default value. If there is no contribute.txt file, all of the defaults are
          used:
        </p>
        <!-- A phone stacks each setting: its name and default on one line, what
             it is under them. -->
        <dl class="space-y-3 text-xs sm:hidden" data-testid="contribute-settings">
          {#each SETTINGS as setting}
            <div class="space-y-0.5 border-b border-border/50 pb-3">
              <dt class="flex flex-wrap items-baseline gap-x-2">
                <span class="font-mono font-medium">{setting.name}</span>
                <span class="text-muted-foreground"
                  >default <span class:font-mono={setting.mono}>{setting.default}</span></span
                >
              </dt>
              <dd>{setting.what}</dd>
            </div>
          {/each}
        </dl>
        <table class="table hidden text-xs sm:table">
          <thead>
            <tr><th>Setting</th><th>Default</th><th>What it is</th></tr>
          </thead>
          <tbody>
            {#each SETTINGS as setting}
              <tr>
                <td class="font-mono">{setting.name}</td>
                <td class:font-mono={setting.mono}>{setting.default}</td>
                <td>{setting.what}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <p class="text-muted-foreground">
          Put contribute.txt in MAGPIE's top level directory. Don't add a <code class="rounded bg-muted px-1">uuid</code> line yourself: when
          MAGPIE is given an anonymous identity it adds one, creating contribute.txt if there isn't
          one, and uses it from then on. Here's an example contribute.txt with every setting:
        </p>
        <pre class="overflow-x-auto rounded-md bg-muted p-4 text-xs" data-testid="contribute-example"><code
            >{EXAMPLE}</code
          ></pre>
      </li>
      <li class="space-y-2">
        <h3 class="font-medium">Run the contribute command</h3>
        <p class="text-muted-foreground">
          From the top level directory, run
          <code class="rounded bg-muted px-1">./bin/magpie contribute</code>. It will run
          continuously until you stop it with Ctrl-C (or until it completes
          <code class="rounded bg-muted px-1">maxtasks</code> tasks if a nonzero value was
          specified).
        </p>
      </li>
    </ol>
  </div>
</section>
