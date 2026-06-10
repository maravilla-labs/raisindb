<script lang="ts">
  import { connection } from '../stores/connection.svelte';
  import { notifications } from '../stores/notifications.svelte';
  import { session } from '../stores/session.svelte';
  import { tasks } from '../stores/tasks.svelte';
</script>

<header class="header">
  <div class="header-left">
    <h1 class="app-title">Shiftboard</h1>
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
    <!-- Native (non-enhanced) form post to the ?/logout action: clears the
         httpOnly cookies and the browser does a full page load of the
         response, which also tears down the WS client and all rune-store
         state (like the old SPA's location.reload()). -->
    <form method="POST" action="?/logout">
      <button type="submit" class="signout">Sign out</button>
    </form>
  </div>
</header>
