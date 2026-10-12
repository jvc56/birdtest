<script lang="ts">
  /**
   * A job page's header: the job's title, then each player on a line of its
   * own -- its seat in its colour, its config's name, and what it does -- then
   * the type, lexicon and variant. A job without a title is headed by its
   * type. The players wait for the job's config; the slot is for the page's
   * own link beside the title.
   */
  import type { JobStats } from '$lib/api';
  import { jobTypeLabel } from '$lib/format';
  import { playerSummary, unusedPlayerSettings, type JobConfig } from '$lib/jobSettings';
  import JobStatusBadge from './JobStatusBadge.svelte';

  export let job: JobStats['job'];
  export let config: JobConfig | null;

  // Whole class names, for Tailwind to find.
  const SEATS = [
    { badge: 'bg-player-1', name: 'text-player-1' },
    { badge: 'bg-player-2', name: 'text-player-2' }
  ];

  $: title = job.title.trim();
  $: unused = config ? unusedPlayerSettings(config) : new Set<string>();
  $: players = config?.players ?? [];
  // A games or pairs job's two players have seats; a lone player has none.
  $: seated = players.length === 2;
</script>

<header class="space-y-2.5" data-testid="job-header">
  <div class="flex flex-wrap items-center gap-3">
    <h1 class="text-2xl font-semibold [overflow-wrap:anywhere]">{title || jobTypeLabel(job.job_type)}</h1>
    <JobStatusBadge status={job.status} />
    <slot />
  </div>
  {#if players.length}
    <ul class="space-y-1" data-testid="job-players">
      {#each players as player, i}
        <li class="flex min-w-0 items-baseline gap-2.5">
          {#if seated}
            <span
              class="shrink-0 rounded px-1.5 font-mono text-xs font-semibold text-background {SEATS[i].badge}"
              aria-label="player {i + 1}">{i + 1}</span
            >
          {/if}
          <div class="min-w-0">
            <a
              href="/player-configs/{player.id}"
              class="font-semibold [overflow-wrap:anywhere] {seated ? SEATS[i].name : 'text-foreground'}"
              >{player.name}</a
            >
            <span class="text-sm text-muted-foreground">{playerSummary(player, unused)}</span>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
  <p class="text-sm text-muted-foreground">
    {#if title}<span>{jobTypeLabel(job.job_type)}</span> · {/if}{job.lexicon ?? '—'} · {job.variant ?? '—'}
  </p>
</header>
