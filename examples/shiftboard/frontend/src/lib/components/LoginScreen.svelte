<script lang="ts">
  import { enhance } from '$app/forms';

  // Action failure data from the ?/login form action (+page.server.ts).
  let { form }: { form?: { error?: string; email?: string } | null } = $props();

  let busy = $state(false);
</script>

<div class="login-screen">
  <!-- Server-side login: posts to the ?/login form action, which calls
       POST /auth/{repo}/login and stores the tokens in httpOnly cookies.
       Works without JavaScript; use:enhance just adds the busy state. -->
  <form
    class="login-card"
    method="POST"
    action="?/login"
    use:enhance={() => {
      busy = true;
      return async ({ update }) => {
        await update();
        busy = false;
      };
    }}
  >
    <h1>Shiftboard</h1>
    <p class="login-hint">Weekend shift planning, with an AI planner on the side.</p>

    <label>
      Email
      <!-- Prefilled demo credentials (created by examples/shiftboard/setup.mjs). -->
      <input
        type="email"
        name="email"
        value={form?.email ?? 'planner@example.com'}
        autocomplete="username"
        required
      />
    </label>
    <label>
      Password
      <input
        type="password"
        name="password"
        value="Planner12345!"
        autocomplete="current-password"
        required
      />
    </label>

    {#if form?.error}
      <p class="login-error" role="alert">{form.error}</p>
    {/if}

    <button type="submit" disabled={busy}>
      {busy ? 'Signing in…' : 'Sign in'}
    </button>
  </form>
</div>
