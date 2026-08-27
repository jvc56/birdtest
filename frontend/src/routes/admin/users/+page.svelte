<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Page } from '$lib/api';
  import { datetime } from '$lib/format';
  import Pagination from '$lib/components/Pagination.svelte';

  let result: Page<Record<string, any>> | null = null;
  let error = '';
  let page = 0;

  async function load(next: number) {
    page = next;
    result = await api.users(next);
  }
  onMount(() => load(0));

  async function remove(user: Record<string, any>) {
    error = '';
    if (
      !confirm(
        `Delete ${user.username}? Their claims and results are removed and task counters are rolled back.`
      )
    )
      return;
    try {
      await api.deleteUser(user.id);
      await load(page);
    } catch (e) {
      error = (e as Error).message;
    }
  }
</script>

<h1 class="mb-2 text-2xl font-semibold">User accounts</h1>
<p class="mb-4 text-sm text-muted-foreground">
  Contribution stats are shown publicly at <a href="/users">/users</a>. Deleting an account rolls
  back its task counters at the application layer, so tasks it held can be claimed again.
</p>
{#if error}<p class="mb-4 text-destructive">{error}</p>{/if}

{#if !result}
  <p class="text-muted-foreground">Loading…</p>
{:else}
  <div class="card overflow-x-auto p-0">
    <table class="table">
      <thead>
        <tr><th>Username</th><th>Joined</th><th class="text-right">Tasks</th><th></th></tr>
      </thead>
      <tbody>
        {#each result.items as user}
          <tr>
            <td>{user.username}{#if user.is_admin}<span class="ml-2 text-xs text-primary">admin</span>{/if}</td>
            <td>{datetime(user.created_at)}</td>
            <td class="text-right tabular-nums">{Number(user.tasks_completed).toLocaleString()}</td>
            <td class="text-right">
              <button class="btn-destructive" on:click={() => remove(user)}>Delete</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  <Pagination page={result.page} perPage={result.per_page} total={result.total} onChange={load} />
{/if}
