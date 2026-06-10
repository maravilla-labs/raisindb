<script setup lang="ts">
/**
 * Header: connection status dot (useConnection) + inbox bell counter
 * (useSubscription on `${home}/inbox/**` node:created in the
 * raisin:access_control workspace, same as the shiftboard demo).
 */
import { computed, ref } from 'vue';
import type { IdentityUser, EventMessage } from '@raisindb/client';
import { client, db, useConnection, useSubscription } from '../lib/raisin';

const props = defineProps<{ user: IdentityUser }>();
defineEmits<{ (e: 'sign-out'): void }>();

const connection = useConnection(client);

const connLabel = computed(() => {
  if (connection.isReady.value) return 'live';
  if (connection.isConnected.value) return 'connected';
  return connection.state.value;
});

const connClass = computed(() => {
  if (connection.isConnected.value) return 'connected';
  if (connection.state.value === 'reconnecting' || connection.state.value === 'connecting') {
    return 'reconnecting';
  }
  return '';
});

// --- Inbox bell -----------------------------------------------------------
// user.home may or may not carry the workspace prefix depending on the auth
// path; subscription path filters are workspace-relative.
const home = (props.user.home ?? '').replace(/^\/raisin:access_control/, '');
const unread = ref(0);

useSubscription(
  db,
  {
    workspace: 'raisin:access_control',
    path: `${home}/inbox/**`,
    event_types: ['node:created'],
    include_node: true,
  },
  (event: EventMessage) => {
    const payload = event.payload as
      | { node?: { properties?: Record<string, unknown> } }
      | undefined;
    // The user's own outgoing chat messages land under their inbox too
    // (role: 'user') — don't count things the user just did.
    if (payload?.node?.properties?.role === 'user') return;
    unread.value += 1;
  },
);
</script>

<template>
  <header class="header">
    <div class="header-left">
      <h1 class="app-title">Vue Board</h1>
      <span class="conn" data-testid="conn">
        <span class="conn-dot" :class="connClass" data-testid="conn-dot"></span>
        {{ connLabel }}
      </span>
    </div>
    <div class="header-right">
      <button class="bell" title="Inbox notifications" data-testid="bell" @click="unread = 0">
        🔔
        <span v-if="unread > 0" class="bell-badge" data-testid="bell-badge">{{ unread }}</span>
      </button>
      <span class="user-email">{{ user.email }}</span>
      <button class="signout" @click="$emit('sign-out')">Sign out</button>
    </div>
  </header>
</template>
