<script lang="ts">
  /**
   * A games or pairs job's significance test: three tiles -- the verdict, player
   * 1's score with its interval, and the pairs or games played against the
   * job's cap -- then the interval on a scale with an even score in the
   * middle, for a pairs job the pair outcomes it runs on, and what the
   * interval means with this job's own confidence. Nothing for a job that runs
   * no test. The job's record, which is not the test, is the match score box
   * above it.
   */
  import type { JobStats } from '$lib/api';
  import { testState } from '$lib/format';
  import { testPct, testScale, testVerdict } from '$lib/matchTest';
  import { pairOutcomes } from '$lib/charts/pentanomial';
  import StatusPill from './StatusPill.svelte';

  export let stats: JobStats;
  /** The players' config names, player 1 first, when the page has them. */
  export let players: string[] = [];

  $: games = stats.games;
  $: test = games?.test;
  $: names = [players[0] ?? 'Player 1', players[1] ?? 'Player 2'] as [string, string];
  $: verdict = games && test ? testVerdict(testState(stats.job.status, games), { ...games, test }) : null;
  $: scale = test ? testScale(test) : null;
  $: outcomes = games?.pentanomial ? pairOutcomes(games.pentanomial, games.units_completed, names) : [];
  $: unit = games?.unit ?? 'pair';
  // A tick at either end is pinned by its own edge, so neither leaves the chart.
  const anchor = (at: number) => `left: ${at}%; transform: translateX(-${at}%)`;
  const text = ['text-player-1', 'text-player-2'] as const;
  // Player 1's outcomes in its colour, player 2's in its own, a split pair
  // fainter than a sweep, even pairs grey. Whole class names, for Tailwind to find.
  const tones = {
    sweep: ['bg-player-1 text-primary-foreground', 'bg-player-2 text-primary-foreground'],
    split: ['bg-player-1/45 text-foreground', 'bg-player-2/45 text-foreground'],
    even: 'bg-muted-foreground/50 text-foreground'
  } as const;
  const tone = (player: 0 | 1 | null, both: boolean) =>
    player === null ? tones.even : tones[both ? 'sweep' : 'split'][player];
</script>

{#if games && test && verdict && scale}
  <div class="card space-y-4" data-testid="significance-test">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <h2 class="text-lg font-medium">Significance Test</h2>
      <StatusPill
        tone={verdict.player === 0 ? 'player1' : verdict.player === 1 ? 'player2' : verdict.running ? 'amber' : 'grey'}
        icon={verdict.player !== null ? 'check' : verdict.running ? 'spin' : verdict.chip === 'paused' ? 'pause' : 'minus'}
        data-testid="test-chip"
      >
        {verdict.player === null ? verdict.chip : `${names[verdict.player]} better`}
      </StatusPill>
    </div>

    <div class="grid grid-cols-[repeat(auto-fit,minmax(10.5rem,1fr))] gap-2.5">
      <div class="min-w-0 space-y-1 rounded-lg border border-border px-3.5 py-3" data-testid="test-verdict">
        <div class="text-[11px] uppercase tracking-wider text-muted-foreground">Verdict</div>
        <div class="text-xl font-semibold [overflow-wrap:anywhere] {verdict.player === null ? '' : text[verdict.player]}">
          {verdict.player === null ? verdict.word : names[verdict.player]}
        </div>
        <div class="text-xs text-muted-foreground">{verdict.line}</div>
      </div>
      <div class="min-w-0 space-y-1 rounded-lg border border-border px-3.5 py-3" data-testid="test-score">
        <div class="text-[11px] uppercase tracking-wider text-muted-foreground">
          <span class={text[0]}>{names[0]}</span>'s score
        </div>
        <div class="text-2xl font-semibold tabular-nums">{testPct(test.mean)}</div>
        <div class="text-xs tabular-nums text-muted-foreground">
          {test.confidence_pct}% interval {testPct(test.lower)} to {testPct(test.upper)}
        </div>
        {#if games.decided && games.units_completed > games.decided.units}
          <!-- The decision stands; what was in flight then landed after it. -->
          <div class="text-xs tabular-nums text-muted-foreground">
            Over all {games.units_completed.toLocaleString()}; {(games.units_completed - games.decided.units).toLocaleString()}
            finished after it decided.
          </div>
        {/if}
      </div>
      <div class="min-w-0 space-y-1 rounded-lg border border-border px-3.5 py-3" data-testid="test-progress">
        <div class="text-[11px] uppercase tracking-wider text-muted-foreground">{unit === 'pair' ? 'Pairs' : 'Games'}</div>
        <div class="text-2xl font-semibold tabular-nums">{games.units_completed.toLocaleString()}</div>
        <div class="text-xs tabular-nums text-muted-foreground">
          of up to {games.max_units.toLocaleString()}, test acts from {games.min_units.toLocaleString()}
        </div>
        <div class="relative mt-1.5 h-1.5 rounded-full bg-muted" aria-hidden="true">
          <div
            class="absolute inset-y-0 left-0 rounded-full bg-foreground/70"
            style="width: {Math.min(1, games.units_completed / Math.max(games.max_units, 1)) * 100}%"
          ></div>
          <div
            class="absolute -inset-y-[3px] w-0.5 bg-warning"
            style="left: {Math.min(1, games.min_units / Math.max(games.max_units, 1)) * 100}%"
            title="the minimum"
          ></div>
        </div>
      </div>
    </div>

    <div
      class="space-y-1"
      role="img"
      aria-label="{names[0]} scores {testPct(test.mean)}, {test.confidence_pct}% interval {testPct(test.lower)} to {testPct(test.upper)}"
      data-testid="test-bar"
    >
      <div class="relative h-[18px]">
        <div class="absolute inset-y-0 left-0 w-1/2 rounded-l bg-player-2/15"></div>
        <div class="absolute inset-y-0 right-0 w-1/2 rounded-r bg-player-1/15"></div>
        <div class="absolute -inset-y-1 left-1/2 border-l border-dashed border-foreground/55"></div>
        <div
          class="absolute inset-y-1 rounded-sm border border-foreground/70 bg-foreground/30"
          style="left: {scale.lower}%; width: {Math.max(scale.upper - scale.lower, 0.5)}%"
        ></div>
        <div class="absolute -inset-y-[3px] w-[3px] -translate-x-1/2 rounded-sm bg-foreground" style="left: {scale.mean}%"></div>
      </div>
      <div class="relative h-4 text-[11px] tabular-nums text-muted-foreground">
        {#each scale.ticks as tick}
          <span class="absolute top-0 whitespace-nowrap" style={anchor(tick.at)}>{tick.label}</span>
        {/each}
      </div>
      <div class="flex justify-between gap-2 text-xs">
        <span class="{text[1]} [overflow-wrap:anywhere]">← {names[1]} better</span>
        <span class="{text[0]} text-right [overflow-wrap:anywhere]">{names[0]} better →</span>
      </div>
    </div>

    {#if outcomes.length}
      <div class="space-y-2.5" data-testid="pair-outcomes">
        <h3 class="text-sm font-medium">Pair outcomes</h3>
        <!-- Each part grows with its count from nothing; a part under 7% of
             the pairs leaves its number to the key. -->
        <div class="flex h-[26px] gap-0.5 overflow-hidden rounded-md text-xs font-semibold tabular-nums" aria-hidden="true">
          {#each outcomes as o}
            {#if o.count}
              {@const labelled = o.count / Math.max(games.units_completed, 1) > 0.07}
              <!-- A labelled part never narrower than its number; the rest a sliver at least. -->
              <span
                class="flex items-center justify-center whitespace-nowrap {labelled ? 'px-1' : 'min-w-[3px]'} {tone(
                  o.player,
                  o.both
                )}"
                style="flex: {o.count} 1 0">{labelled ? o.count.toLocaleString() : ''}</span
              >
            {/if}
          {/each}
        </div>
        <dl class="grid grid-cols-[1fr_auto_auto] items-center gap-x-3 gap-y-1 text-sm">
          {#each outcomes as o}
            <div class="contents" data-testid="pair-outcome-{o.bucket}">
              <dt class="flex items-center gap-3 [overflow-wrap:anywhere]">
                <span class="h-2.5 w-2.5 shrink-0 rounded-sm {tone(o.player, o.both)}" aria-hidden="true"></span>{o.label}
              </dt>
              <dd class="text-right tabular-nums">{o.count.toLocaleString()}</dd>
              <dd class="text-right tabular-nums text-muted-foreground">{o.share}%</dd>
            </div>
          {/each}
        </dl>
        <p class="text-xs text-muted-foreground">
          The test runs on all {games.units_completed.toLocaleString()} pairs, each scored by its result across
          the pair. Pairs that played identically count as even.{#if games.divergent_pairs !== undefined}{' '}{games.divergent_pairs.toLocaleString()} of {games.units_completed.toLocaleString()} pairs diverged: a
            diagnostic of how often these two configs differ at all, not part of the test.{/if}
        </p>
      </div>
    {/if}

    <details class="text-sm" data-testid="test-explained">
      <summary class="cursor-pointer text-muted-foreground">What does the interval mean?</summary>
      <div class="mt-2 space-y-2 text-muted-foreground">
        <p>
          The test asks whether one player is better than the other, at {test.confidence_pct}%
          confidence. It keeps an interval around player 1's score per game (1 for a win, ½ for a
          draw):
          {#if unit === 'pair'}
            each completed pair is one observation, scored by player 1's result across its two
            games. Pairing the games (the same racks, the seats swapped) cancels much of the luck
            of the draw, so a pairs job decides sooner than one playing as many separate games.
            Pairs whose two games played identically stay in the sample as even pairs: they are
            what makes a paired run lower-variance than an unpaired one.
          {:else}
            each game is one observation.
          {/if}
        </p>
        <p>
          Once the interval lies wholly above 50%, player 1 is better; wholly below, player 2 is.
          Unlike an ordinary confidence interval it stays valid however often it is checked, so
          the job can stop the moment it decides: the chance of naming a winner between two equal
          players is at most about {Math.round((100 - test.confidence_pct) * 10) / 10}%. If
          it has not decided by the job's cap, neither player is shown to be better, and the
          interval says how large a difference the games rule out.
        </p>
      </div>
    </details>
  </div>
{/if}
