<script lang="ts">
  import { connection } from '../stores/connection.svelte';
  import { notifications } from '../stores/notifications.svelte';
  import { session } from '../stores/session.svelte';
  import { tasks } from '../stores/tasks.svelte';
  import { view } from '../stores/view.svelte';
  import { DEMO_ACCOUNTS } from '$lib/demo-accounts';

  let switchForm: HTMLFormElement;
  let switchEmail = $state('');
  let switchPassword = $state('');

  // Quick user-switch: posts straight to the ?/login action (same as the
  // login screen) with the chosen demo account's credentials - overwrites
  // the auth cookies without an explicit sign-out step.
  function switchTo(e: Event) {
    const account = DEMO_ACCOUNTS.find((a) => a.email === (e.target as HTMLSelectElement).value);
    if (!account) return;
    switchEmail = account.email;
    switchPassword = account.password;
    queueMicrotask(() => switchForm.requestSubmit());
  }
</script>

<header class="header">
  <div class="header-left">
    <h1 class="app-title">Shiftboard</h1>
    <!-- The tabs swap only the right column; the board stays visible. -->
    <nav class="tabs" aria-label="View">
      <button
        class="tab"
        class:active={view.tab === 'board'}
        data-testid="tab-board"
        onclick={() => (view.tab = 'board')}
      >
        Board
      </button>
      <button
        class="tab"
        class:active={view.tab === 'planner'}
        data-testid="tab-planner"
        onclick={() => (view.tab = 'planner')}
      >
        Planner
      </button>
    </nav>
    <span class="conn" title="WebSocket connection state">
      <span class="conn-dot {connection.status}"></span>
      {connection.status}
    </span>
  </div>

  <div class="header-right">
    <!-- Bell badge is driven purely by the inbox node subscription.
         Clicking it also scrolls/focuses the task panel when tasks exist. -->
    <button
      class="bell"
      onclick={() => {
        notifications.clear();
        tasks.requestFocus();
      }}
      aria-label="Notifications ({notifications.unread} unread)"
    >
      🔔
      {#if notifications.unread > 0}
        <span class="bell-badge">{notifications.unread}</span>
      {/if}
    </button>

    <span class="user-email">{session.user?.email}</span>

    <!-- Quick-switch between the demo accounts (README "Setup") without a
         manual sign-out step - resubmits ?/login with the chosen creds. -->
    <select
      class="user-switch"
      aria-label="Switch demo account"
      value={session.user?.email}
      onchange={switchTo}
    >
      {#each DEMO_ACCOUNTS as account (account.email)}
        <option value={account.email}>{account.label}</option>
      {/each}
    </select>
    <form method="POST" action="?/login" bind:this={switchForm} hidden>
      <input type="hidden" name="email" value={switchEmail} />
      <input type="hidden" name="password" value={switchPassword} />
    </form>

    <!-- Native (non-enhanced) form post to the ?/logout action: clears the
         httpOnly cookies and the browser does a full page load of the
         response, which also tears down the WS client and all rune-store
         state (like the old SPA's location.reload()). -->
    <form method="POST" action="?/logout">
      <button type="submit" class="signout">Sign out</button>
    </form>
  </div>
</header>
