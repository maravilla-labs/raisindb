<script setup lang="ts">
/**
 * Chat panel, step 1: resolve which conversation to open.
 *
 * Uses useConversationList to find the latest ai_chat conversation with the
 * shift-planner agent (same selection as the shiftboard demo). Once
 * resolved, mounts ChatSession — which calls useConversation in its own
 * setup() — with either the existing conversation path (resume) or null
 * (a new conversation is created lazily on first send).
 */
import { ref, watch } from 'vue';
import type { ConversationListItem } from '@raisindb/client';
import { AGENT_PATH, db, useConversationList } from '../lib/raisin';
import ChatSession from './ChatSession.vue';

const list = useConversationList({ database: db, type: 'ai_chat' });

const resolved = ref(false);
const conversationPath = ref<string | null>(null);

/** agent_ref arrives as a raisin reference object over HTTP; tolerate both shapes. */
function agentRefPath(item: ConversationListItem): string | undefined {
  const ref_ = item.agentRef as unknown;
  if (typeof ref_ === 'string') return ref_;
  if (ref_ && typeof ref_ === 'object') return (ref_ as Record<string, string>)['raisin:path'];
  return undefined;
}

// useConversationList kicks off load() synchronously, so isLoading is true
// when the watcher registers; resolve on the first transition back to false.
watch(
  list.isLoading,
  (loading) => {
    if (loading || resolved.value) return;
    conversationPath.value =
      list.conversations.value
        .filter((c) => agentRefPath(c) === AGENT_PATH)
        .sort((a, b) => (b.updatedAt ?? '').localeCompare(a.updatedAt ?? ''))[0]
        ?.conversationPath ?? null;
    resolved.value = true;
  },
  { immediate: true },
);
</script>

<template>
  <section class="panel chat-panel" data-testid="chat">
    <h2 class="panel-title">Planning chat</h2>
    <p v-if="!resolved" class="muted">Opening conversation…</p>
    <ChatSession v-else :conversation-path="conversationPath" />
  </section>
</template>
