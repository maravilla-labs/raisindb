<script lang="ts">
  import { Minus, X, Send, Bot, Loader2, Plus, MessageSquare, PenSquare, Bug } from 'lucide-svelte';
  import {
    aiChatStore,
    activeAIConversation,
    activeAIMessages,
    aiAgents,
    aiConversations,
    isWaitingForAIResponse,
    DEFAULT_AGENT,
    type AIMessage,
  } from '$lib/stores/ai-chat';
  import AIChatMessage from './AIChatMessage.svelte';
  import AgentPicker from './AgentPicker.svelte';
  import AIConversationList from './AIConversationList.svelte';

  interface Props {
    isMinimized: boolean;
  }

  let { isMinimized }: Props = $props();

  let messageInput = $state('');
  let sending = $state(false);
  let messagesContainer: HTMLDivElement | null = $state(null);
  let inputElement: HTMLTextAreaElement | null = $state(null);
  let showAgentPicker = $state(false);
  let showConversationList = $state(false);
  let showDebug = $state(false);

  const conversation = $derived($activeAIConversation);
  const messages = $derived($activeAIMessages);
  const agents = $derived($aiAgents);
  const conversations = $derived(Array.from($aiConversations.values()));
  const conversationCount = $derived(conversations.length);
  const isWaiting = $derived($isWaitingForAIResponse);

  // Auto-scroll to bottom when new messages arrive
  $effect(() => {
    if (messages.length > 0 && messagesContainer && !isMinimized) {
      setTimeout(() => scrollToBottom(), 50);
    }
  });

  // Focus input when opened
  $effect(() => {
    if (!isMinimized && inputElement) {
      setTimeout(() => inputElement?.focus(), 100);
    }
  });

  function scrollToBottom() {
    if (messagesContainer) {
      messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }
  }

  async function handleSend() {
    if (!messageInput.trim() || sending || isWaiting) return;

    // Create conversation if needed
    if (!conversation) {
      const convId = await aiChatStore.createConversation(DEFAULT_AGENT.path);
      if (!convId) {
        console.error('Failed to create conversation');
        return;
      }
    }

    sending = true;
    const content = messageInput.trim();
    messageInput = '';

    try {
      await aiChatStore.sendMessage(content);
    } finally {
      sending = false;
      setTimeout(() => inputElement?.focus(), 0);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  function handleMinimize() {
    aiChatStore.minimize();
  }

  function handleClose() {
    aiChatStore.close();
  }

  function handleRestore() {
    aiChatStore.restore();
  }

  async function handleNewChat() {
    showConversationList = false;
    showAgentPicker = true;
  }

  async function handleAgentSelect(agentPath: string) {
    showAgentPicker = false;
    await aiChatStore.createConversation(agentPath);
  }

  function handleShowConversations() {
    showConversationList = true;
  }

  async function handleSelectConversation(convId: string) {
    showConversationList = false;
    await aiChatStore.setActiveConversation(convId);
  }

  async function handleDeleteConversation(convId: string) {
    await aiChatStore.deleteConversation(convId);
  }

  function handleCloseConversationList() {
    showConversationList = false;
  }

  function getAgentName(): string {
    if (conversation?.agentRef) {
      const path = conversation.agentRef['raisin:path'];
      return path?.split('/').pop() || 'Assistant';
    }
    return 'Assistant';
  }

  function toggleDebug() {
    showDebug = !showDebug;
  }
</script>

{#if isMinimized}
  <!-- Minimized bar -->
  <div class="minimized-bar">
    <button class="minimized-content" onclick={handleRestore}>
      <Bot size={16} class="text-purple-400" />
      <span class="minimized-name">
        AI {getAgentName()}
      </span>
      {#if isWaiting}
        <Loader2 size={14} class="animate-spin text-green-400" />
      {/if}
    </button>
    <button class="action-button" onclick={handleClose}>
      <X size={16} />
    </button>
  </div>
{:else}
  <!-- Full popup -->
  <div class="chat-popup">
    <!-- Header -->
    <div class="popup-header">
      <div class="header-info">
        <button class="history-button" onclick={handleShowConversations} title="All conversations">
          <MessageSquare size={16} />
          {#if conversationCount > 0}
            <span class="conversation-badge">{conversationCount}</span>
          {/if}
        </button>
        <span class="header-name">
          {conversation ? getAgentName() : 'AI Assistant'}
        </span>
        {#if isWaiting}
          <Loader2 size={14} class="animate-spin" />
        {/if}
      </div>
      <div class="header-actions">
        <button class="new-chat-button" onclick={handleNewChat} title="Start new conversation">
          <PenSquare size={14} />
          <span>New</span>
        </button>
        <button
          class="action-button"
          class:debug-active={showDebug}
          onclick={toggleDebug}
          title="Toggle debug panel"
        >
          <Bug size={16} />
        </button>
        <button class="action-button" onclick={handleMinimize} title="Minimize">
          <Minus size={16} />
        </button>
        <button class="action-button" onclick={handleClose} title="Close">
          <X size={16} />
        </button>
      </div>
    </div>

    <!-- Conversation List -->
    {#if showConversationList}
      <AIConversationList
        {conversations}
        activeConversationId={conversation?.id ?? null}
        onSelect={handleSelectConversation}
        onNewChat={handleNewChat}
        onDelete={handleDeleteConversation}
        onClose={handleCloseConversationList}
      />
    <!-- Agent Picker -->
    {:else if showAgentPicker}
      <AgentPicker
        {agents}
        onSelect={handleAgentSelect}
        onCancel={() => showAgentPicker = false}
      />
    {:else}
      <!-- Messages -->
      <div class="messages-container" bind:this={messagesContainer}>
        {#if messages.length === 0 && !conversation}
          <div class="welcome-screen">
            <div class="welcome-icon">
              <Bot size={32} />
            </div>
            <h3>AI Assistant</h3>
            <p>Start a new conversation or continue an existing one</p>
            <div class="welcome-actions">
              <button class="welcome-new-chat" onclick={handleNewChat}>
                <PenSquare size={16} />
                <span>New Chat</span>
              </button>
              {#if conversationCount > 0}
                <button class="welcome-history" onclick={handleShowConversations}>
                  <MessageSquare size={16} />
                  <span>History ({conversationCount})</span>
                </button>
              {/if}
            </div>
            <p class="welcome-hint">Or type a message below to start chatting</p>
          </div>
        {:else if messages.length === 0}
          <div class="empty-messages">
            <Bot size={32} class="text-purple-300 mb-2" />
            <p>Start a conversation</p>
            <p class="text-xs text-zinc-500 mt-1">Type a message below</p>
          </div>
        {:else}
          {#each messages as msg (msg.id)}
            <AIChatMessage message={msg} />
          {/each}
          {#if isWaiting}
            <div class="typing-indicator">
              <Bot size={16} class="text-purple-400" />
              <span class="dots">
                <span></span>
                <span></span>
                <span></span>
              </span>
            </div>
          {/if}
        {/if}
      </div>

      <!-- Input -->
      <div class="input-container">
        <textarea
          bind:this={inputElement}
          bind:value={messageInput}
          onkeydown={handleKeydown}
          placeholder="Ask the AI..."
          disabled={sending}
          rows="1"
        ></textarea>
        <button
          class="send-button"
          onclick={handleSend}
          disabled={!messageInput.trim() || sending || isWaiting}
        >
          {#if sending || isWaiting}
            <Loader2 size={16} class="animate-spin" />
          {:else}
            <Send size={16} />
          {/if}
        </button>
      </div>

      <!-- Debug Panel -->
      {#if showDebug}
        <div class="debug-panel">
          <div class="debug-header">
            <span class="debug-title">Debug: Messages ({messages.length})</span>
            <span class="debug-status">
              waiting: {isWaiting ? 'true' : 'false'}
            </span>
          </div>
          <div class="debug-content">
            <pre>{JSON.stringify(messages, null, 2)}</pre>
          </div>
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .minimized-bar {
    width: 220px;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px 12px 0 0;
    box-shadow: 0 -2px 10px rgba(0, 0, 0, 0.1);
    padding: 0.5rem 0.75rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    cursor: pointer;
    transition: background 0.2s;
  }

  .minimized-bar:hover {
    background: #f9fafb;
  }

  .minimized-content {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    flex: 1;
    background: transparent;
    border: none;
    padding: 0;
    cursor: pointer;
  }

  .minimized-name {
    font-weight: 500;
    font-size: 0.875rem;
    color: #1f2937;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chat-popup {
    width: 360px;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px 12px 0 0;
    box-shadow: 0 -4px 20px rgba(0, 0, 0, 0.15);
    display: flex;
    flex-direction: column;
    max-height: 500px;
    animation: popupSlideUp 0.2s ease-out;
  }

  @keyframes popupSlideUp {
    from {
      opacity: 0;
      transform: translateY(20px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .popup-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.625rem 0.75rem;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    border-radius: 12px 12px 0 0;
    color: white;
  }

  .header-info {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }

  .history-button {
    position: relative;
    padding: 0.375rem;
    background: rgba(255, 255, 255, 0.15);
    border: none;
    border-radius: 6px;
    color: white;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.2s;
  }

  .history-button:hover {
    background: rgba(255, 255, 255, 0.25);
  }

  .conversation-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    background: #22c55e;
    color: white;
    font-size: 10px;
    font-weight: 600;
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .new-chat-button {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.375rem 0.625rem;
    background: rgba(255, 255, 255, 0.2);
    border: none;
    border-radius: 6px;
    color: white;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .new-chat-button:hover {
    background: rgba(255, 255, 255, 0.3);
  }

  .header-name {
    font-weight: 600;
    font-size: 0.875rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .action-button {
    padding: 0.375rem;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: rgba(255, 255, 255, 0.8);
    cursor: pointer;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .minimized-bar .action-button {
    color: #6b7280;
  }

  .action-button:hover {
    background: rgba(255, 255, 255, 0.2);
    color: white;
  }

  .minimized-bar .action-button:hover {
    background: #e5e7eb;
    color: #374151;
  }

  .messages-container {
    flex: 1;
    overflow-y: auto;
    padding: 0.75rem;
    min-height: 250px;
    max-height: 350px;
    background: #fafafa;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .empty-messages {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #9ca3af;
    font-size: 0.875rem;
    text-align: center;
    padding: 2rem;
  }

  .welcome-screen {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
    padding: 1.5rem;
    gap: 0.75rem;
  }

  .welcome-icon {
    width: 56px;
    height: 56px;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border-radius: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 0.25rem;
  }

  .welcome-screen h3 {
    margin: 0;
    font-size: 1.125rem;
    font-weight: 600;
    color: #1f2937;
  }

  .welcome-screen > p {
    margin: 0;
    font-size: 0.875rem;
    color: #6b7280;
  }

  .welcome-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .welcome-new-chat {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.625rem 1rem;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .welcome-new-chat:hover {
    transform: scale(1.02);
    box-shadow: 0 4px 12px rgba(139, 92, 246, 0.4);
  }

  .welcome-history {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.625rem 1rem;
    background: white;
    color: #6b7280;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .welcome-history:hover {
    background: #f9fafb;
    border-color: #d1d5db;
    color: #374151;
  }

  .welcome-hint {
    margin-top: 0.5rem;
    font-size: 0.75rem;
    color: #9ca3af;
  }

  .typing-indicator {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    background: white;
    border-radius: 12px;
    width: fit-content;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .dots {
    display: flex;
    gap: 4px;
  }

  .dots span {
    width: 6px;
    height: 6px;
    background: #8b5cf6;
    border-radius: 50%;
    animation: bounce 1.4s infinite ease-in-out both;
  }

  .dots span:nth-child(1) { animation-delay: -0.32s; }
  .dots span:nth-child(2) { animation-delay: -0.16s; }

  @keyframes bounce {
    0%, 80%, 100% { transform: scale(0.8); opacity: 0.5; }
    40% { transform: scale(1); opacity: 1; }
  }

  .input-container {
    border-top: 1px solid #e5e7eb;
    padding: 0.625rem;
    display: flex;
    align-items: flex-end;
    gap: 0.5rem;
    background: white;
  }

  .input-container textarea {
    flex: 1;
    padding: 0.5rem 0.875rem;
    font-size: 0.875rem;
    border: 1px solid #e5e7eb;
    border-radius: 16px;
    outline: none;
    transition: all 0.2s;
    resize: none;
    min-height: 36px;
    max-height: 120px;
    font-family: inherit;
  }

  .input-container textarea:focus {
    border-color: #8b5cf6;
    box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.1);
  }

  .input-container textarea:disabled {
    background: #f3f4f6;
  }

  .send-button {
    padding: 0.5rem;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border: none;
    border-radius: 50%;
    cursor: pointer;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .send-button:hover:not(:disabled) {
    transform: scale(1.05);
    box-shadow: 0 2px 8px rgba(139, 92, 246, 0.4);
  }

  .send-button:disabled {
    background: #d1d5db;
    cursor: not-allowed;
  }

  /* Animation classes via Tailwind-like utilities */
  :global(.animate-spin) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  :global(.text-purple-400) {
    color: #a78bfa;
  }

  :global(.text-purple-300) {
    color: #c4b5fd;
  }

  :global(.text-green-400) {
    color: #4ade80;
  }

  :global(.text-zinc-500) {
    color: #71717a;
  }

  :global(.mb-2) {
    margin-bottom: 0.5rem;
  }

  /* Debug Panel */
  .debug-active {
    background: rgba(234, 179, 8, 0.3) !important;
    color: #fbbf24 !important;
  }

  .debug-panel {
    border-top: 1px solid #fbbf24;
    background: #1a1a1a;
    max-height: 200px;
    display: flex;
    flex-direction: column;
  }

  .debug-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.375rem 0.625rem;
    background: #262626;
    border-bottom: 1px solid #333;
  }

  .debug-title {
    font-size: 0.6875rem;
    font-weight: 600;
    color: #fbbf24;
    font-family: 'SF Mono', Menlo, Monaco, monospace;
  }

  .debug-status {
    font-size: 0.625rem;
    color: #9ca3af;
    font-family: 'SF Mono', Menlo, Monaco, monospace;
  }

  .debug-content {
    flex: 1;
    overflow: auto;
    padding: 0.5rem;
  }

  .debug-content pre {
    margin: 0;
    font-size: 0.625rem;
    line-height: 1.4;
    color: #d1d5db;
    font-family: 'SF Mono', Menlo, Monaco, monospace;
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
