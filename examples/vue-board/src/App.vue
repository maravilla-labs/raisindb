<script setup lang="ts">
/**
 * App shell: restore the stored session (useAuth.initSession), then show
 * either the login screen or the board + chat layout.
 *
 * The board/chat components are mounted only while authenticated, so their
 * composables (useSql / useSubscription / useConversation*) run against an
 * authenticated WebSocket and clean up via onUnmounted on logout.
 */
import { onMounted, ref } from 'vue';
import { client, REPOSITORY, useAuth } from './lib/raisin';
import LoginScreen from './components/LoginScreen.vue';
import AppHeader from './components/AppHeader.vue';
import BoardPanel from './components/BoardPanel.vue';
import ChatPanel from './components/ChatPanel.vue';

const auth = useAuth(client);
const booting = ref(true);

onMounted(async () => {
  try {
    // Restores tokens from localStorage, reconnects + re-authenticates the
    // WebSocket; resolves null when there is no stored session.
    await auth.initSession(REPOSITORY);
  } catch (err) {
    console.warn('[app] session restore failed, showing login:', err);
  } finally {
    booting.value = false;
  }
});

async function signOut() {
  try {
    await auth.logout({ disconnect: false, reconnect: true });
  } catch (err) {
    console.warn('[app] logout failed:', err);
  }
}
</script>

<template>
  <div v-if="booting" class="splash">Loading…</div>

  <LoginScreen v-else-if="!auth.isAuthenticated.value || !auth.user.value" :auth="auth" />

  <div v-else class="app">
    <AppHeader :user="auth.user.value" @sign-out="signOut" />
    <main class="layout">
      <BoardPanel />
      <ChatPanel />
    </main>
  </div>
</template>
