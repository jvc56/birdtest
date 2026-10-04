<script lang="ts">
  /**
   * Two players side by side, a column each: in every row the higher value is
   * green and the lower red (equal values neither), with the same said in
   * words for a screen reader. Every row is one where more is better for the
   * player who has it.
   */
  import { standings, type CompareRow } from '$lib/compare';

  /** The players' names, player 1 first. */
  export let players: [string, string];
  export let rows: CompareRow[];
  /** Heads the label column. */
  export let caption = '';

  const tint = {
    higher: 'bg-success/15 font-semibold text-success',
    lower: 'bg-destructive/10 text-destructive'
  } as const;
</script>

<div class="overflow-x-auto">
  <table class="table text-sm" data-testid="player-compare">
    <thead>
      <tr>
        <th class="w-1/3">{caption}</th>
        {#each players as player}
          <th class="text-right [overflow-wrap:anywhere]">{player}</th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each rows as row}
        {@const marks = standings(row.numbers)}
        <tr>
          <td class="text-muted-foreground" title={row.title}>{row.label}</td>
          {#each row.values as value, i}
            {@const mark = marks[i]}
            <td class="text-right tabular-nums {mark ? tint[mark] : ''}" data-standing={mark ?? 'even'}>
              {value}{#if mark}<span class="sr-only"> ({mark})</span>{/if}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</div>
