<script lang="ts">
  /**
   * A state as a pill: a small icon, then its word, in its tone (see
   * `lib/status.ts`). The icon is drawn inline, and hidden from a screen
   * reader, which reads the word. A spinning arc stands still for anyone who
   * has asked for less motion.
   */
  import type { StatusIcon, StatusTone } from '$lib/status';

  export let tone: StatusTone;
  export let icon: StatusIcon;
  /** A tooltip, for a state that needs a sentence. */
  export let title: string | undefined = undefined;

  // Whole class names, for Tailwind to find.
  const TONES: Record<StatusTone, string> = {
    green: 'bg-success/15 text-success border-success/30',
    amber: 'bg-warning/15 text-warning border-warning/30',
    red: 'bg-destructive/15 text-destructive border-destructive/30',
    grey: 'bg-muted text-muted-foreground border-border',
    blue: 'bg-primary/15 text-primary border-primary/30',
    player1: 'bg-player-1/15 text-player-1 border-player-1/40',
    player2: 'bg-player-2/15 text-player-2 border-player-2/40'
  };
</script>

<span
  class="inline-flex items-center gap-1 whitespace-nowrap rounded-full border px-2 py-0.5 text-xs font-medium {TONES[tone]}"
  {title}
  {...$$restProps}
>
  <svg
    class="h-3 w-3 shrink-0 {icon === 'spin' ? 'animate-spin motion-reduce:animate-none' : ''}"
    viewBox="0 0 12 12"
    aria-hidden="true"
    data-icon={icon}
  >
    {#if icon === 'play'}
      <path d="M4 2.5v7l6-3.5z" fill="currentColor" />
    {:else if icon === 'pause'}
      <rect x="3" y="2.5" width="2.2" height="7" rx=".6" fill="currentColor" />
      <rect x="6.8" y="2.5" width="2.2" height="7" rx=".6" fill="currentColor" />
    {:else if icon === 'check'}
      <path d="M2.5 6.3l2.3 2.3 4.7-5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    {:else if icon === 'warn'}
      <path d="M6 1.6l4.8 8.4H1.2z" fill="currentColor" />
      <path d="M6 4.6v2.4M6 8.4v.1" stroke="hsl(222 22% 11%)" stroke-width="1.3" stroke-linecap="round" />
    {:else if icon === 'spark'}
      <path d="M6 1.5l1.2 3.3 3.3 1.2-3.3 1.2L6 10.5 4.8 7.2 1.5 6l3.3-1.2z" fill="currentColor" />
    {:else if icon === 'spin'}
      <circle cx="6" cy="6" r="4" fill="none" stroke="currentColor" stroke-opacity=".3" stroke-width="1.7" />
      <path d="M6 2a4 4 0 0 1 4 4" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
    {:else if icon === 'clock'}
      <circle cx="6" cy="6" r="4.2" fill="none" stroke="currentColor" stroke-width="1.4" />
      <path d="M6 3.6V6l1.7 1.1" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
    {:else if icon === 'x'}
      <path d="M3.3 3.3l5.4 5.4M8.7 3.3L3.3 8.7" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
    {:else}
      <path d="M3 6h6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
    {/if}
  </svg>
  <slot />
</span>
