<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type JobListItem } from '$lib/api';
  import { jobTypeLabel } from '$lib/format';
  import JobStatusBadge from '$lib/components/JobStatusBadge.svelte';

  let active: JobListItem[] = [];

  onMount(async () => {
    const jobs = await api.jobs();
    active = jobs.items.filter((job) => job.status === 'active');
  });
</script>

<section class="space-y-8">
  <div class="space-y-3">
    <h1 class="text-3xl font-semibold">Crowdsourced word game analysis</h1>
    <p class="max-w-2xl text-muted-foreground">
      birdtest distributes word game research across contributors' machines. Admins define jobs —
      opening rack analysis, autoplay games, matched game pairs, leave generation — and workers
      claim tasks, run them locally with MAGPIE, and submit results. No account is required to
      contribute.
    </p>
    <div class="flex gap-3">
      <a href="/jobs" class="btn-primary no-underline hover:no-underline">Browse jobs</a>
      <a href="/register" class="btn-secondary no-underline hover:no-underline">Create an account</a>
    </div>
  </div>

  <div class="card space-y-3">
    <h2 class="text-lg font-medium">Contribute</h2>
    <p class="text-sm text-muted-foreground">
      You need only <a href="https://github.com/jvc56/MAGPIE">MAGPIE</a> — no Python, no Docker.
      Put a <code class="rounded bg-muted px-1">contribute.txt</code> beside it:
    </p>
    <pre class="overflow-x-auto rounded-md bg-muted p-4 text-xs"><code
        >server   {typeof window !== 'undefined' ? window.location.origin : ''}
threads  7
maxtasks 0</code
      ></pre>
    <p class="text-sm text-muted-foreground">
      then run <code class="rounded bg-muted px-1">magpie contribute</code>. Add an
      <code class="rounded bg-muted px-1">apikey</code> line to attribute your work to your account
      instead of an anonymous UUID.
    </p>
  </div>

  {#if active.length}
    <div class="space-y-3">
      <h2 class="text-lg font-medium">Active jobs</h2>
      <div class="grid gap-3 sm:grid-cols-2">
        {#each active as job}
          <a href="/jobs/{job.id}" class="card no-underline hover:border-primary/50 hover:no-underline">
            <div class="flex items-center justify-between">
              <span class="font-medium text-foreground">{jobTypeLabel(job.job_type)}</span>
              <JobStatusBadge status={job.status} />
            </div>
            <p class="mt-1 text-sm text-muted-foreground">
              priority {job.priority} · {job.allocation ?? 0}% allocation
            </p>
          </a>
        {/each}
      </div>
    </div>
  {/if}
</section>
