<script lang="ts">
  import { Minus, X, Send, MessageCircle } from 'lucide-svelte';
  import { chatStore } from '$lib/stores/chat';
  import {
    messagingStore,
    messages as messagesStore,
    conversations,
    type Conversation,
    type Message
  } from '$lib/stores/messaging-store';
  import { user } from '$lib/stores/auth';
  import { presenceStore } from '$lib/stores/presence';
  import ChatMessage from './ChatMessage.svelte';

  interface Props {
    conversationId: string;
    isMinimized: boolean;
  }

  let { conversationId, isMinimized }: Props = $props();

  const currentUser = $derived($user);
  const currentUserId = $derived(currentUser?.id || '');

  // Get conversation from store reactively
  const conversation = $derived($conversations.get(conversationId));

  // Check if participant is online
  const isParticipantOnline = $derived(
    conversation?.participantId ? presenceStore.isOnline(conversation.participantId) : false
  );

  // Get messages reactively from store (Same as Inbox page)
  const conversationMessages = $derived(
    conversationId ? ($messagesStore.get(conversationId) || []) : []
  );

  let messageInput = $state('');
  let sending = $state(false);
  let messagesContainer: HTMLDivElement | null = $state(null);
  let inputElement: HTMLInputElement | null = $state(null);

  // Load messages when component mounts or conversation changes
  $effect(() => {
    if (conversationId) {
      messagingStore.loadConversationMessages(conversationId);
    }
  });

  // Auto-scroll to bottom when new messages arrive
  $effect(() => {
    if (conversationMessages.length > 0 && messagesContainer && !isMinimized) {
      setTimeout(() => scrollToBottom(), 50);
    }
  });

  function scrollToBottom() {
    if (messagesContainer) {
      messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }
  }

  // Mark as read when popup is opened
  $effect(() => {
    if (!isMinimized && conversation) {
      // Mark conversation as read
      if (conversation.unreadCount > 0) {
        messagingStore.markAsRead(conversationId);
      }
    }
  });

  async function handleSend() {
    if (!messageInput.trim() || sending || !conversation) return;

    sending = true;
    const content = messageInput.trim();
    messageInput = '';

    try {
      // Use messagingStore for optimistic UI
      const success = await messagingStore.sendMessage(conversationId, content);
      if (!success) {
        messageInput = content; // Restore on error
      } else {
        // Force refresh after delay to ensure consistency if WebSocket is slow
        setTimeout(() => messagingStore.loadConversationMessages(conversationId), 1000);
      }
    } catch (err) {
      messageInput = content; // Restore on error
    } finally {
      sending = false;
      // Focus back on input
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
    chatStore.minimizeConversation(conversationId);
  }

  function handleClose() {
    chatStore.closeConversation(conversationId);
  }

  function handleRestore() {
    chatStore.restoreConversation(conversationId);
  }

  function isMyMessage(msg: Message): boolean {
    return msg.properties.sender_id === currentUserId;
  }
</script>

{#if conversation}
  {#if isMinimized}
    <!-- Minimized bar -->
    <div class="minimized-bar">
      <button class="minimized-content" onclick={handleRestore}>
        <span class="online-dot" class:offline={!isParticipantOnline}></span>
        <span class="minimized-name">
          {conversation.participantDisplayName}
        </span>
        {#if conversation.unreadCount > 0}
          <span class="minimized-badge">{conversation.unreadCount}</span>
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
          <span class="online-dot" class:offline={!isParticipantOnline}></span>
          <span class="header-name">
            {conversation.participantDisplayName}
          </span>
        </div>
        <div class="header-actions">
          <button class="action-button" onclick={handleMinimize} title="Minimize">
            <Minus size={16} />
          </button>
          <button class="action-button" onclick={handleClose} title="Close">
            <X size={16} />
          </button>
        </div>
      </div>

      <!-- Messages -->
      <div class="messages-container" bind:this={messagesContainer}>
        {#if conversationMessages.length === 0}
          <div class="empty-messages">
            {#if conversationMessages === undefined}
              Loading...
            {:else}
              No messages yet
            {/if}
          </div>
        {:else}
          {#each conversationMessages as msg (msg.id)}
            <ChatMessage
              message={msg}
              isMine={isMyMessage(msg)}
              senderDisplayName={conversation.participantDisplayName}
            />
          {/each}
        {/if}
      </div>

      <!-- Input -->
      <div class="input-container">
        <input
          type="text"
          bind:this={inputElement}
          bind:value={messageInput}
          onkeydown={handleKeydown}
          placeholder="Type a message..."
          disabled={sending}
        />
        <button
          class="send-button"
          onclick={handleSend}
          disabled={!messageInput.trim() || sending}
        >
          <Send size={16} />
        </button>
      </div>
    </div>
  {/if}
{/if}

<style>
  .minimized-bar {
    width: 240px;
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

  .minimized-badge {
    background: #ef4444;
    color: white;
    font-size: 0.625rem;
    font-weight: 700;
    padding: 0.125rem 0.375rem;
    border-radius: 999px;
    min-width: 16px;
    text-align: center;
  }

  .online-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #22c55e;
    flex-shrink: 0;
  }

  .online-dot.offline {
    background: #9ca3af;
  }

  .chat-popup {
    width: 320px;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px 12px 0 0;
    box-shadow: 0 -4px 20px rgba(0, 0, 0, 0.15);
    display: flex;
    flex-direction: column;
    max-height: 450px;
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
    background: linear-gradient(135deg, #6366f1, #4f46e5);
    border-radius: 12px 12px 0 0;
  }

  .header-info {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }

  .header-info .online-dot {
    background: #4ade80;
    box-shadow: 0 0 0 2px rgba(74, 222, 128, 0.3);
  }

  .header-name {
    font-weight: 600;
    font-size: 0.875rem;
    color: white;
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
    min-height: 200px;
    max-height: 300px;
    background: #fafafa;
    display: flex;
    flex-direction: column;
  }

  .empty-messages {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #9ca3af;
    font-size: 0.875rem;
  }

  .input-container {
    border-top: 1px solid #e5e7eb;
    padding: 0.625rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: white;
  }

  .input-container input {
    flex: 1;
    padding: 0.5rem 0.875rem;
    font-size: 0.875rem;
    border: 1px solid #e5e7eb;
    border-radius: 999px;
    outline: none;
    transition: all 0.2s;
  }

  .input-container input:focus {
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .input-container input:disabled {
    background: #f3f4f6;
  }

  .send-button {
    padding: 0.5rem;
    background: linear-gradient(135deg, #6366f1, #4f46e5);
    color: white;
    border: none;
    border-radius: 50%;
    cursor: pointer;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .send-button:hover:not(:disabled) {
    transform: scale(1.05);
    box-shadow: 0 2px 8px rgba(99, 102, 241, 0.4);
  }

  .send-button:disabled {
    background: #d1d5db;
    cursor: not-allowed;
  }
</style>
