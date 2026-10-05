<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type JobListItem } from '$lib/api';
  import { jobTitle, jobTypeLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';

  let active: JobListItem[] = [];

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
    <h1 class="text-3xl font-semibold">Crowdsourced Crossword Game Analysis</h1>
    <p class="max-w-2xl text-muted-foreground">
      birdtest is a distributed task queue for analyzing crossword games. Admins create jobs and
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
        <h3 class="font-medium">Create a contribute.txt file</h3>
        <p class="text-muted-foreground">
          Put a <code class="rounded bg-muted px-1">contribute.txt</code> in the directory you run
          MAGPIE from, the one holding its <code class="rounded bg-muted px-1">data/</code>. Each
          line is a setting's name and its value:
        </p>
        <div class="overflow-x-auto">
          <table class="table text-xs">
            <thead>
              <tr><th>Setting</th><th>Required</th><th>What it is</th></tr>
            </thead>
            <tbody>
              <tr>
                <td class="font-mono">server</td><td>yes</td>
                <td>This site's address, as above.</td>
              </tr>
              <tr>
                <td class="font-mono">apikey</td><td>no</td>
                <td>An API key from your account, to contribute under your username (step 3).</td>
              </tr>
              <tr>
                <td class="font-mono">threads</td><td>no</td>
                <td>How many threads to run tasks on. Leave it out to use every core but one.</td>
              </tr>
              <tr>
                <td class="font-mono">maxtasks</td><td>no</td>
                <td>How many tasks to run before stopping. 0, or left out, runs until you stop it.</td>
              </tr>
              <tr>
                <td class="font-mono">idlewait</td><td>no</td>
                <td>Seconds to wait before asking again when there is no work. 5 if left out.</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p class="text-muted-foreground">
          Don't add a <code class="rounded bg-muted px-1">uuid</code> line yourself: MAGPIE adds one
          when it is given an anonymous identity, and uses it from then on. Here's an example contribute.txt
          that you can copy into the MAGPIE directory to get started:
        </p>
        <pre class="overflow-x-auto rounded-md bg-muted p-4 text-xs"><code
            >server   {typeof window !== 'undefined' ? window.location.origin : ''}</code
          ></pre>
      </li>
      <li class="space-y-2">
        <h3 class="font-medium">(Optional) Create an account</h3>
        <p class="text-muted-foreground">
          To contribute under a username, <a href="/register">create an account</a>, make an API
          key on your account page, and add its <code class="rounded bg-muted px-1">apikey</code>
          line to your contribute.txt. Otherwise you contribute anonymously, with nothing to sign up
          for.
        </p>
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
