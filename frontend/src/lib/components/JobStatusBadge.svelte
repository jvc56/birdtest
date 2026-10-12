<script lang="ts">
  /** A job's status as a pill: active, inactive or completed, each with its icon. */
  import { jobStatus } from '$lib/status';
  import StatusPill from './StatusPill.svelte';

  export let status: string;

  $: shown = jobStatus(status);
</script>

<StatusPill tone={shown.tone} icon={shown.icon}>{shown.word}</StatusPill>
