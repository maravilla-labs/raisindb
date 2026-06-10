<script setup lang="ts">
/**
 * Chat panel, step 2: the live conversation (useConversation).
 *
 * With a conversationPath the store resumes it (persistent SSE subscription
 * + history via loadMessages); without one, createOptions makes
 * sendMessage() create the conversation with the agent on first send.
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { AGENT_PATH, db, useConversation } from '../lib/raisin';

const props = defineProps<{ conversationPath: string | null }>();

const chat = useConversation({
  database: db,
  conversationPath: props.conversationPath ?? undefined,
  createOptions: { participant: AGENT_PATH },
});

onMounted(() => {
  if (props.conversationPath) {
    chat.loadMessages().catch((err) => {
      console.warn('[chat] loading history failed:', err);
    });
  }
});

const input = ref('');
const scroller = ref<HTMLElement | null>(null);

// Only user/assistant messages with text render as bubbles; tool/system
// messages surface via the tool-call badges instead.
const visibleMessages = computed(() =>
  chat.messages.value.filter(
    (m) => (m.role === 'user' || m.role === 'assistant') && m.content,
  ),
);

// NOTE: isWaiting must NOT feed this indicator — the store sets
// isWaiting=true at end-of-turn ("waiting for user input"), so including it
// shows a permanent "thinking…" bubble after every reply.
const thinking = computed(
  () =>
    chat.isStreaming.value &&
    !chat.streamingText.value &&
    chat.activeToolCalls.value.length === 0,
);

// Keep the chat scrolled to the bottom as messages/stream/tool calls change.
watch(
  [
    () => visibleMessages.value.length,
    () => chat.streamingText.value,
    () => chat.activeToolCalls.value.length,
  ],
  async () => {
    await nextTick();
    scroller.value?.scrollTo({ top: scroller.value.scrollHeight });
  },
);

function send() {
  const text = input.value.trim();
  if (!text || chat.isStreaming.value) return;
  input.value = '';
  chat.sendMessage(text).catch((err) => {
    console.warn('[chat] send failed:', err);
  });
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    send();
  }
}
</script>

<template>
  <div class="chat-messages" ref="scroller" data-testid="chat-messages">
    <p
      v-if="visibleMessages.length === 0 && !chat.isStreaming.value"
      class="muted chat-empty"
    >
      Ask the shift planner anything — e.g. “Which shifts are still open?”
    </p>

    <div
      v-for="(message, i) in visibleMessages"
      :key="message.id ?? i"
      class="bubble"
      :class="message.role"
      :data-testid="`bubble-${message.role}`"
    >
      {{ message.content }}
    </div>

    <!-- Live streaming text, before it lands as a persisted message. -->
    <div
      v-if="chat.isStreaming.value && chat.streamingText.value"
      class="bubble assistant streaming"
      data-testid="streaming"
    >
      {{ chat.streamingText.value }}
    </div>

    <!-- Tool calls in flight: 🔧 name + spinner, then ✓/✗. -->
    <div v-if="chat.activeToolCalls.value.length > 0" class="tool-calls">
      <span
        v-for="tool in chat.activeToolCalls.value"
        :key="tool.id"
        class="tool-badge"
        :class="tool.status"
        data-testid="tool-badge"
      >
        🔧 {{ tool.functionName }}
        <span v-if="tool.status === 'running'" class="spinner"></span>
        <template v-else-if="tool.status === 'completed'">✓</template>
        <template v-else>✗</template>
      </span>
    </div>

    <div v-if="thinking" class="bubble assistant thinking">thinking…</div>
  </div>

  <p v-if="chat.error.value" class="error-banner" role="alert">{{ chat.error.value }}</p>

  <div class="chat-input">
    <input
      v-model="input"
      type="text"
      placeholder="Message the shift planner…"
      data-testid="chat-input"
      :disabled="chat.isStreaming.value"
      @keydown="onKeydown"
    />
    <button
      data-testid="chat-send"
      :disabled="chat.isStreaming.value || !input.trim()"
      @click="send"
    >
      Send
    </button>
  </div>
</template>
