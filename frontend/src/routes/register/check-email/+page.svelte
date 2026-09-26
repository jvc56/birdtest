<script lang="ts">
  // The development hint is for a developer's own stack, not every registrant.
  const dev = import.meta.env.DEV;
</script>

<div class="mx-auto max-w-md text-center">
  <h1 class="mb-4 text-2xl font-semibold">Check your email</h1>
  <p class="text-muted-foreground">
    We sent a confirmation link to the address you registered with. Open it to activate your
    account, then sign in.
  </p>
  {#if dev}
    <p class="mt-6 text-sm text-muted-foreground">
      Running locally with <code class="rounded bg-muted px-1">MAIL_BACKEND=console</code>? The link
      is in the backend's stdout log rather than an inbox.
    </p>
  {/if}
  <a href="/login" class="btn-secondary mt-6 inline-flex no-underline hover:no-underline">
    Go to sign in
  </a>
</div>
