<script lang="ts">
  import { user } from '$lib/stores/auth';
  import {
    messagingStore,
    messages as messagesStore,
    type Message
  } from '$lib/stores/messaging-store';
  import { goto } from '$app/navigation';
  import {
    ArrowLeft,
    MessageCircle,
    Send,
    AlertCircle
  } from 'lucide-svelte';
  import ChatMessage from '$lib/components/ChatMessage.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  const currentUserId = $derived($user?.id || '');

  // Get conversation from data (loaded in +page.ts)
  const conversation = $derived(data.conversation);
  const conversationId = $derived(data.conversationId);

  // Get messages - prefer store for real-time updates, fall back to page data for initial load
  const conversationMessages = $derived(
    conversationId
      ? ($messagesStore.get(conversationId) || data.messages || [])
      : []
  );

  // UI state
  let replyContent = $state('');
  let sendingReply = $state(false);
  let messagesContainer: HTMLDivElement | null = $state(null);
  let replyInputElement: HTMLInputElement | null = $state(null);

  // Mark as read when viewing
  $effect(() => {
    if (conversation && conversation.unreadCount > 0) {
      messagingStore.markAsRead(conversationId);
    }
  });

  // Auto-scroll when messages change
  $effect(() => {
    if (conversationMessages.length > 0 && messagesContainer) {
      setTimeout(() => scrollToBottom(), 50);
    }
  });

  function scrollToBottom() {
    if (messagesContainer) {
      messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }
  }

  function isMyMessage(msg: Message): boolean {
    return msg.properties.sender_id === currentUserId;
  }

  async function handleSendReply() {
    if (!conversation || !replyContent.trim() || sendingReply) return;

    sendingReply = true;
    const content = replyContent.trim();
    replyContent = '';

    const success = await messagingStore.sendMessage(conversationId, content);

    if (!success) {
      replyContent = content; // Restore on error
    }

    sendingReply = false;
    setTimeout(() => replyInputElement?.focus(), 0);
  }

  function handleReplyKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSendReply();
    }
  }

  function handleBack() {
    goto('/inbox');
  }
</script>

<svelte:head>
  <title>{conversation?.participantDisplayName || 'Conversation'} - Inbox - Launchpad</title>
</svelte:head>

<div class="conversation-page">
  {#if data.error}
    <div class="error-state">
      <AlertCircle size={48} />
      <h3>{data.error}</h3>
      <button class="btn-back-link" onclick={handleBack}>
        <ArrowLeft size={16} />
        Back to Inbox
      </button>
    </div>
  {:else if conversation}
    <div class="conversation-view">
      <!-- Header -->
      <div class="conversation-header">
        <button class="btn-back" onclick={handleBack} title="Back to inbox">
          <ArrowLeft size={18} />
        </button>
        <div class="conversation-title">
          <div class="participant-avatar">
            {conversation.participantDisplayName.charAt(0).toUpperCase()}
          </div>
          <div>
            <h3>{conversation.participantDisplayName}</h3>
          </div>
        </div>
      </div>

      <!-- Messages -->
      <div class="conversation-messages" bind:this={messagesContainer}>
        {#if conversationMessages.length === 0}
          <div class="empty-conversation">
            <MessageCircle size={32} />
            <p>No messages yet. Start the conversation!</p>
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
      <div class="conversation-input">
        <input
          type="text"
          bind:this={replyInputElement}
          bind:value={replyContent}
          onkeydown={handleReplyKeydown}
          placeholder="Type a message..."
          disabled={sendingReply}
        />
        <button
          class="btn-send-reply"
          onclick={handleSendReply}
          disabled={!replyContent.trim() || sendingReply}
        >
          <Send size={18} />
        </button>
      </div>
    </div>
  {:else}
    <div class="loading-state">
      Loading conversation...
    </div>
  {/if}
</div>

<style>
  .conversation-page {
    max-width: 800px;
    margin: 0 auto;
    padding: 2rem;
    height: calc(100vh - 80px);
    display: flex;
    flex-direction: column;
  }

  .error-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 4rem 2rem;
    text-align: center;
    color: #6b7280;
  }

  .error-state h3 {
    margin: 1rem 0;
    color: #374151;
  }

  .btn-back-link {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: #f3f4f6;
    color: #374151;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-back-link:hover {
    background: #e5e7eb;
  }

  .loading-state {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4rem 2rem;
    color: #9ca3af;
  }

  .conversation-view {
    display: flex;
    flex-direction: column;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    overflow: hidden;
    flex: 1;
    min-height: 0;
  }

  .conversation-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1rem;
    border-bottom: 1px solid #e5e7eb;
    background: #f9fafb;
    flex-shrink: 0;
  }

  .btn-back {
    padding: 0.5rem;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: #6b7280;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-back:hover {
    background: #e5e7eb;
    color: #374151;
  }

  .conversation-title {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .participant-avatar {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: linear-gradient(135deg, #6366f1, #8b5cf6);
    color: white;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: 1rem;
  }

  .conversation-title h3 {
    margin: 0;
    font-size: 0.9375rem;
    font-weight: 600;
    color: #1f2937;
  }

  .participant-email {
    font-size: 0.75rem;
    color: #6b7280;
  }

  .conversation-messages {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    background: #fafafa;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .empty-conversation {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #9ca3af;
  }

  .empty-conversation :global(svg) {
    margin-bottom: 0.5rem;
  }

  .conversation-input {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-top: 1px solid #e5e7eb;
    background: white;
    flex-shrink: 0;
  }

  .conversation-input input {
    flex: 1;
    padding: 0.625rem 1rem;
    border: 1px solid #e5e7eb;
    border-radius: 999px;
    font-size: 0.875rem;
    outline: none;
    transition: border-color 0.2s;
  }

  .conversation-input input:focus {
    border-color: #6366f1;
  }

  .btn-send-reply {
    padding: 0.625rem;
    background: #6366f1;
    color: white;
    border: none;
    border-radius: 50%;
    cursor: pointer;
    transition: background 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .btn-send-reply:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-send-reply:disabled {
    background: #d1d5db;
    cursor: not-allowed;
  }

  @media (max-width: 640px) {
    .conversation-page {
      padding: 1rem;
      height: calc(100vh - 60px);
    }
  }
</style>
