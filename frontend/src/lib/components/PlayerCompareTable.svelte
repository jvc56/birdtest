<script lang="ts">
  /**
   * Two players side by side, a column each: in every row the better value is
   * green and the worse red (equal values neither), with the same said in
   * words for a screen reader. More is better unless the row says otherwise.
   */
  import { standings, type CompareRow } from '$lib/compare';

  /** The players' names, player 1 first. */
  export let players: [string, string];
  export let rows: CompareRow[];
  /** Heads the label column. */
  export let caption = '';

  const tint = {
    better: 'bg-success/15 font-semibold text-success',
    worse: 'bg-destructive/10 text-destructive'
  } as const;
</script>

<!-- Not the site's `.table`: compact, rounded cells set apart by a gap, in
     a rounded frame, rather than full-width ruled rows. -->
<div class="max-w-2xl overflow-x-auto rounded-xl border border-border p-1">
  <table class="w-full border-separate border-spacing-1 text-sm" data-testid="player-compare">
    <thead>
      <tr>
        <th class="w-1/3 px-3 py-1 text-left text-xs font-medium text-muted-foreground">{caption}</th>
        {#each players as player}
          <th class="px-3 py-1 text-right text-xs font-medium text-muted-foreground [overflow-wrap:anywhere]"
            >{player}</th
          >
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row}
        {@const marks = standings(row.numbers, row.better)}
        <tr>
          <td class="rounded-lg px-3 py-1 text-muted-foreground" title={row.title}>{row.label}</td>
          {#each row.values as value, i}
            {@const mark = marks[i]}
            <td
              class="rounded-lg px-3 py-1 text-right tabular-nums {mark ? tint[mark] : 'bg-muted/40'}"
              data-standing={mark ?? 'even'}
            >
              {value}{#if mark}<span class="sr-only"> ({mark})</span>{/if}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>
