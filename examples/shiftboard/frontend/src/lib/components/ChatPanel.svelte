<script lang="ts">
  import type { AgentChatState } from '../stores/chat.svelte';

  let {
    chat,
    title = 'Planning chat',
    placeholder = 'Message the agent…',
    emptyHint = 'Ask the agent anything.',
    testid = 'chat',
  }: {
    chat: AgentChatState;
    title?: string;
    placeholder?: string;
    emptyHint?: string;
    testid?: string;
  } = $props();

  let input = $state('');
  let scroller = $state<HTMLElement>();

  // Only user/assistant messages with text are rendered as bubbles;
  // tool/system messages are surfaced via the tool-call badges instead.
  // Plan/task lifecycle messages (messageType ai_plan / ai_task_update —
  // their content is just the plan/task title) belong to the plan panel,
  // not the transcript.
  const visibleMessages = $derived(
    chat.snapshot.messages.filter(
      (m) =>
        (m.role === 'user' || m.role === 'assistant') &&
        m.content &&
        m.messageType !== 'ai_plan' &&
        m.messageType !== 'ai_task_update',
    ),
  );

  const thinking = $derived(
    chat.snapshot.isStreaming &&
      !chat.snapshot.streamingText &&
      chat.snapshot.activeToolCalls.length === 0,
  );

  // Keep the chat scrolled to the bottom as messages/stream/tool calls change.
  $effect(() => {
    void visibleMessages.length;
    void chat.snapshot.streamingText;
    void chat.snapshot.activeToolCalls.length;
    scroller?.scrollTo({ top: scroller.scrollHeight });
  });

  function send() {
    const text = input.trim();
    if (!text || chat.snapshot.isStreaming) return;
    input = '';
    chat.send(text);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }
</script>

<section class="panel chat-panel" data-testid={testid}>
  <h2 class="panel-title">{title}</h2>

  <div class="chat-messages" bind:this={scroller}>
    <!-- SSR already renders the history below; only show the placeholder
         when there is nothing to show yet. -->
    {#if !chat.ready && visibleMessages.length === 0}
      <p class="muted">Opening conversation…</p>
    {:else if chat.ready && visibleMessages.length === 0 && !chat.snapshot.isStreaming}
      <p class="muted chat-empty">{emptyHint}</p>
    {/if}

    {#each visibleMessages as message, i (message.id ?? i)}
      <div class="bubble {message.role}" data-testid="{testid}-bubble-{message.role}">
        {message.content}
      </div>
    {/each}

    <!-- Live streaming text, before it lands as a persisted message. -->
    {#if chat.snapshot.isStreaming && chat.snapshot.streamingText}
      <div class="bubble assistant streaming">
        {chat.snapshot.streamingText}
      </div>
    {/if}

    <!-- Tool calls in flight: 🔧 name + spinner, then ✓/✗. -->
    {#if chat.snapshot.activeToolCalls.length > 0}
      <div class="tool-calls">
        {#each chat.snapshot.activeToolCalls as tool (tool.id)}
          <span class="tool-badge {tool.status}">
            🔧 {tool.functionName}
            {#if tool.status === 'running'}
              <span class="spinner"></span>
            {:else if tool.status === 'completed'}
              ✓
            {:else}
              ✗
            {/if}
          </span>
        {/each}
      </div>
    {/if}

    {#if thinking}
      <div class="bubble assistant thinking">thinking…</div>
    {/if}
  </div>

  {#if chat.snapshot.error}
    <p class="error-banner" role="alert">{chat.snapshot.error}</p>
  {/if}

  <div class="chat-input">
    <input
      type="text"
      {placeholder}
      data-testid="{testid}-input"
      bind:value={input}
      onkeydown={onKeydown}
      disabled={!chat.ready || chat.snapshot.isStreaming}
    />
    <button
      data-testid="{testid}-send"
      onclick={send}
      disabled={!chat.ready || chat.snapshot.isStreaming || !input.trim()}
    >
      Send
    </button>
  </div>
</section>
