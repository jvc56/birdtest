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
  const isDefaultServer = origin === DEFAULT_SERVER;

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
    <p class="max-w-2xl text-muted-foreground">
      birdtest runs MAGPIE, a crossword board game engine, on volunteers' computers to play test
      matches between versions, tune its settings and study openings. Admins create jobs and
      contributors' machines claim their tasks, run them locally with MAGPIE, and submit the
      results.
    </p>
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
              {#if job.name}{jobTypeLabel(job.job_type)} · {/if}{job.allocation ?? 0}% allocation
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
          Otherwise you contribute anonymously, with nothing to sign up for.
        </p>
      </li>
      <li class="space-y-2">
        <h3 class="font-medium">{isDefaultServer ? '(Optional) ' : ''}Create a contribute.txt file</h3>
        <p class="text-muted-foreground">
          {#if isDefaultServer}
            MAGPIE works without one: it contributes to this site anonymously, on every core but one.
            To change that, or to contribute with the API key from step 2, put a
          {:else}
            This site is not MAGPIE's default server, so tell it where to contribute: put a
          {/if}
          <code class="rounded bg-muted px-1">contribute.txt</code> in the directory you run MAGPIE
          from, the one holding its <code class="rounded bg-muted px-1">data/</code>. Each line is a
          setting's name and its value, and any setting left out takes its default:
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
          Don't add a <code class="rounded bg-muted px-1">uuid</code> line yourself: when MAGPIE is
          given an anonymous identity it adds one, creating contribute.txt if there isn't one, and
          uses it from then on. Here's an example contribute.txt
          {isDefaultServer ? 'that runs on 4 threads:' : 'for this site:'}
        </p>
        <pre class="overflow-x-auto rounded-md bg-muted p-4 text-xs"><code
            >{isDefaultServer ? 'threads  4' : `server   ${origin}`}</code
          ></pre>
      </li>
      <li class="space-y-2">
        <h3 class="font-medium">Run the contribute command</h3>
        <p class="text-muted-foreground">
          From the same directory, run <code class="rounded bg-muted px-1">./bin/magpie contribute</code>.
          It claims tasks, runs them and submits the results until you stop it with Ctrl-C (or
          until <code class="rounded bg-muted px-1">maxtasks</code>).
        </p>
      </li>
    </ol>
  </div>
</section>
