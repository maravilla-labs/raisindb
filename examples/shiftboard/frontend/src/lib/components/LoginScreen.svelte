<script lang="ts">
  import { enhance } from '$app/forms';
  import { DEMO_ACCOUNTS } from '$lib/demo-accounts';

  // Action failure data from the ?/login form action (+page.server.ts).
  let { form }: { form?: { error?: string; email?: string } | null } = $props();

  let busy = $state(false);
  let email = $state(form?.email ?? DEMO_ACCOUNTS[0].email);
  let password = $state(DEMO_ACCOUNTS[0].password);

  function applyDemoAccount(e: Event) {
    const chosen = DEMO_ACCOUNTS.find((a) => a.email === (e.target as HTMLSelectElement).value);
    if (chosen) {
      email = chosen.email;
      password = chosen.password;
    }
  }
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
      Demo account
      <select onchange={applyDemoAccount}>
        {#each DEMO_ACCOUNTS as account (account.email)}
          <option value={account.email} selected={account.email === email}>
            {account.label} — {account.email}
          </option>
        {/each}
      </select>
    </label>

    <label>
      Email
      <!-- Prefilled demo credentials (created by examples/shiftboard/setup.mjs). -->
      <input type="email" name="email" bind:value={email} autocomplete="username" required />
    </label>
    <label>
      Password
      <input
        type="password"
        name="password"
        bind:value={password}
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
