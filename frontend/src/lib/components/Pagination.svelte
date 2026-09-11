<script lang="ts">
  export let page: number;
  export let perPage: number;
  export let total: number;
  export let onChange: (page: number) => void;

  // `-1` is the server's marker for "an exact count is not worth computing";
  // paging still works, we just cannot say how many pages there are.
  $: hasNext = total < 0 ? true : (page + 1) * perPage < total;
</script>

<div class="mt-4 flex items-center gap-3 text-sm">
  <button class="btn-secondary" disabled={page === 0} on:click={() => onChange(page - 1)}>
    Previous
  </button>
  <span class="text-muted-foreground">
    Page {page + 1}{#if total >= 0} of {Math.max(1, Math.ceil(total / perPage))}{/if}
  </span>
  <button class="btn-secondary" disabled={!hasNext} on:click={() => onChange(page + 1)}>
    Next
  </button>
</div>
