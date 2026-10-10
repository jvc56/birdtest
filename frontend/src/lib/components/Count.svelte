<script lang="ts">
  /**
   * A count to its last digit that a narrow column wraps only between its
   * digit groups -- "1,234,567,<wbr>890" -- never inside one. Its text is the
   * count as `exactCount` writes it.
   */
  import { exactCount } from '$lib/format';

  export let value: number | null;

  $: groups = exactCount(value).split(',');
</script>

{#each groups as group, i}{#if i},<wbr />{/if}{group}{/each}
