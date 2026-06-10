<script lang="ts">
  import type { PageProps } from './$types';
  import { session } from '$lib/stores/session.svelte';
  import { board } from '$lib/stores/board.svelte';
  import { notifications } from '$lib/stores/notifications.svelte';
  import { chat } from '$lib/stores/chat.svelte';
  import { tasks } from '$lib/stores/tasks.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import { initClient } from '$lib/raisin';
  import LoginScreen from '$lib/components/LoginScreen.svelte';
  import Header from '$lib/components/Header.svelte';
  import ShiftBoard from '$lib/components/ShiftBoard.svelte';
  import ChatPanel from '$lib/components/ChatPanel.svelte';
  import TaskPanel from '$lib/components/TaskPanel.svelte';
  import Toasts from '$lib/components/Toasts.svelte';

  let { data, form }: PageProps = $props();

  // Seed the rune stores from SSR data. The plain statements run during the
  // server render (so the HTML is complete) and again on hydration (so the
  // first client paint matches it) — no client fetch, no flash.
  function seedStores() {
    session.user = data.session?.user ?? null;
    if (data.session && data.board) {
      board.seed(data.board);
    }
    if (data.session && data.chat) {
      chat.seed(data.chat);
    }
    if (data.session && data.tasks) {
      tasks.seed(data.tasks);
    }
  }
  seedStores();

  // Re-seed when `data` changes client-side without a full page load
  // (e.g. the enhanced login form redirecting back to `/`).
  // $effect.pre runs before the DOM updates and never during SSR.
  $effect.pre(() => {
    void data;
    seedStores();
  });

  // After hydration: bring up the live layer — WS client (JWT from the
  // cookie session), board subscription, inbox bell, streaming chat.
  // ($effect never runs during SSR.)
  $effect(() => {
    if (!data.session) return;
    const { token, user } = data.session;
    const chatData = data.chat ?? { conversationPath: null, messages: [] };

    (async () => {
      try {
        await initClient(token);
        connection.init();
        chat.connect(chatData);
        tasks.connect();
        await Promise.all([board.connect(), notifications.init(user)]);
      } catch (err) {
        console.error('[shiftboard] live wiring failed:', err);
        const msg = err instanceof Error ? err.message : String(err);
        // A stale session cookie can pass the HTTP-side check but fail the
        // WebSocket JWT authentication. One full reload runs the server-side
        // refresh hook and hands us a fresh token; if it still fails, fall
        // back to a clean re-login instead of a silently broken page.
        if (/token|auth|websocket/i.test(msg)) {
          const KEY = 'shiftboard:ws-auth-retry';
          if (!sessionStorage.getItem(KEY)) {
            sessionStorage.setItem(KEY, '1');
            window.location.reload();
            return;
          }
          sessionStorage.removeItem(KEY);
          // logout is a POST form action - submit it programmatically
          const f = document.createElement('form');
          f.method = 'POST';
          f.action = '?/logout';
          document.body.appendChild(f);
          f.submit();
          return;
        }
        board.error = msg;
      }
    })();
  });
</script>

{#if !data.session}
  <LoginScreen {form} />
{:else}
  <div class="app">
    <Header />
    <main class="layout">
      <ShiftBoard />
      <div class="side">
        <TaskPanel />
        <ChatPanel />
      </div>
    </main>
  </div>
  <Toasts />
{/if}
