<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { getDatabase } from '$lib/raisin';
  import { chat } from '$lib/stores/active-conversation.svelte';
  import { inbox } from '$lib/stores/inbox-store.svelte';
  import { goto } from '$app/navigation';
  import {
    ArrowLeft,
    MessageCircle,
    Send,
    AlertCircle,
    Pencil,
    Check,
    X
  } from 'lucide-svelte';
  import ChatMessage from '$lib/components/ChatMessage.svelte';
  import type { PageData } from './$types';
  import type { ChatMessage as SDKChatMessage } from '@raisindb/client';

  let { data }: { data: PageData } = $props();

  const currentUserId = $derived($user?.id || '');
  const myNodeId = $derived(data.myWorkspaceNodeId || '');

  // Get conversation metadata from SSR data
  const conversation = $derived(data.conversation);
  const conversationId = $derived(data.conversationId);

  // Use SDK messages as primary source, fall back to SSR data while loading
  const displayMessages = $derived.by(() => {
    const live = chat.messages;
    if (live && live.length > 0) return live;
    // Fall back to SSR-loaded messages, mapped to ChatMessage shape
    return (data.messages || []).map((row: any) => ({
      role: row.properties?.role || (row.properties?.sender_id === currentUserId || row.properties?.sender_id === myNodeId ? 'user' : 'assistant'),
      content: row.properties?.content || (typeof row.properties?.body === 'string' ? row.properties.body : row.properties?.body?.content) || row.properties?.data?.content || '',
      timestamp: row.properties?.created_at || '',
      id: row.id,
      path: row.path,
      senderId: row.properties?.sender_id,
      senderDisplayName: row.properties?.sender_display_name,
      status: row.properties?.status,
      messageType: row.properties?.message_type,
      data: row.properties?.data,
      children: [],
    } satisfies SDKChatMessage));
  });

  // Subject editing
  const displaySubject = $derived(conversation?.subject || '');

  // UI state
  let replyContent = $state('');
  let sendingReply = $state(false);
  let messagesContainer: HTMLDivElement | null = $state(null);
  let replyInputElement: HTMLInputElement | null = $state(null);
  let editingSubject = $state(false);
  let editSubjectValue = $state('');

  // Open the conversation using the SDK ConversationStore when data is ready
  $effect(() => {
    const convPath = data.conversationPath;
    if (!convPath) return;

    let cancelled = false;
    (async () => {
      try {
        const db = await getDatabase();
        if (cancelled) return;
        await chat.open(db, convPath);
      } catch (err) {
        console.error('[inbox/[id]] Failed to open conversation via SDK:', err);
      }
    })();

    return () => {
      cancelled = true;
      chat.close();
    };
  });

  // Mark as read when viewing (uses inbox rune store)
  $effect(() => {
    const convPath = data.conversationPath;
    if (conversation && conversation.unreadCount > 0 && convPath) {
      inbox.markAsRead(convPath);
    }
  });

  // Auto-scroll when messages change
  $effect(() => {
    if (displayMessages.length > 0 && messagesContainer) {
      setTimeout(() => scrollToBottom(), 50);
    }
  });

  // Also scroll when streaming text updates
  $effect(() => {
    if (chat.streamingText && messagesContainer) {
      setTimeout(() => scrollToBottom(), 50);
    }
  });

  function scrollToBottom() {
    if (messagesContainer) {
      messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }
  }

  function isMyMessage(msg: SDKChatMessage): boolean {
    if (msg.role === 'user') return true;
    if (msg.senderId === currentUserId || msg.senderId === myNodeId) return true;
    return false;
  }

  async function handleSendReply() {
    if (!conversation || !replyContent.trim() || sendingReply) return;

    sendingReply = true;
    const content = replyContent.trim();
    replyContent = '';

    try {
      await chat.sendMessage(content);
    } catch (err) {
      console.error('[inbox/[id]] sendMessage failed:', err);
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

  function startEditSubject() {
    editSubjectValue = displaySubject || '';
    editingSubject = true;
  }

  async function saveSubject() {
    // Subject editing not yet supported via SDK ConversationStore;
    // keeping the UI elements for future implementation.
    editingSubject = false;
  }

  function cancelEditSubject() {
    editingSubject = false;
  }

  function handleSubjectKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      saveSubject();
    } else if (e.key === 'Escape') {
      cancelEditSubject();
    }
  }
</script>

<svelte:head>
  <title>{displaySubject && displaySubject !== 'Chat' ? displaySubject : conversation?.participantDisplayName || 'Conversation'} - Inbox - Launchpad</title>
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
          <div class="conversation-title-text">
            <h3>{conversation.participantDisplayName}</h3>
            {#if editingSubject}
              <div class="subject-edit">
                <input
                  type="text"
                  class="subject-input"
                  bind:value={editSubjectValue}
                  onkeydown={handleSubjectKeydown}
                  placeholder="Conversation title..."
                />
                <button class="subject-btn save" onclick={saveSubject} title="Save">
                  <Check size={14} />
                </button>
                <button class="subject-btn cancel" onclick={cancelEditSubject} title="Cancel">
                  <X size={14} />
                </button>
              </div>
            {:else}
              <button class="subject-display" onclick={startEditSubject} title="Edit title">
                <span class="subject-text">
                  {displaySubject && displaySubject !== 'Chat' ? displaySubject : 'Add title...'}
                </span>
                <Pencil size={12} />
              </button>
            {/if}
          </div>
        </div>
      </div>

      <!-- Messages -->
      <div class="conversation-messages" bind:this={messagesContainer}>
        {#if displayMessages.length === 0 && !chat.isStreaming}
          <div class="empty-conversation">
            <MessageCircle size={32} />
            <p>No messages yet. Start the conversation!</p>
          </div>
        {:else}
          {#each displayMessages as msg (msg.id || msg.timestamp)}
            <ChatMessage
              message={msg}
              isMine={isMyMessage(msg)}
              senderDisplayName={conversation.participantDisplayName}
              plans={chat.plans}
              onApprovePlan={chat.approvePlan}
              onRejectPlan={chat.rejectPlan}
            />
          {/each}
          {#if chat.isStreaming || chat.activeToolCalls.length > 0}
            <div class="assistant-activity">
              {#if chat.streamingText}
                <ChatMessage
                  message={{ role: 'assistant', content: chat.streamingText, timestamp: new Date().toISOString() }}
                  isMine={false}
                  senderDisplayName={conversation.participantDisplayName}
                  streaming={true}
                />
              {:else if chat.activeToolCalls.length === 0}
                <div class="thinking-indicator">
                  <span class="dot"></span>
                  <span class="dot"></span>
                  <span class="dot"></span>
                </div>
              {/if}
              {#if chat.activeToolCalls.length > 0}
                <div class="tool-calls-inline">
                  {#each chat.activeToolCalls as tc (tc.id)}
                    <div class="tool-call-item" class:running={tc.status === 'running'} class:completed={tc.status === 'completed'} class:failed={tc.status === 'failed'}>
                      {#if tc.status === 'running'}
                        <span class="tool-spinner"></span>
                      {:else if tc.status === 'completed'}
                        <Check size={14} />
                      {:else}
                        <AlertCircle size={14} />
                      {/if}
                      <span class="tool-fn-name">{tc.functionName}</span>
                      {#if tc.durationMs}
                        <span class="tool-duration">{tc.durationMs}ms</span>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        {/if}
      </div>

      <!-- Error display -->
      {#if chat.error}
        <div class="conversation-error">
          <AlertCircle size={14} />
          {chat.error}
        </div>
      {/if}

      <!-- Input -->
      <div class="conversation-input">
        <input
          type="text"
          bind:this={replyInputElement}
          bind:value={replyContent}
          onkeydown={handleReplyKeydown}
          placeholder="Type a message..."
          disabled={sendingReply || chat.isStreaming}
        />
        <button
          class="btn-send-reply"
          onclick={handleSendReply}
          disabled={!replyContent.trim() || sendingReply || chat.isStreaming}
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
    color: var(--color-text-muted);
  }

  .error-state h3 {
    margin: 1rem 0;
    color: var(--color-text-secondary);
  }

  .btn-back-link {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: var(--color-surface);
    color: var(--color-text-secondary);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: border-color 0.2s, color 0.2s;
  }

  .btn-back-link:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  .loading-state {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4rem 2rem;
    color: var(--color-text-muted);
  }

  .conversation-view {
    display: flex;
    flex-direction: column;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    overflow: hidden;
    flex: 1;
    min-height: 0;
  }

  .conversation-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1rem;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-elevated);
    flex-shrink: 0;
  }

  .btn-back {
    padding: 0.5rem;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-back:hover {
    background: var(--color-surface);
    color: var(--color-accent);
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
    background: linear-gradient(135deg, var(--color-accent), var(--color-rose));
    color: var(--color-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: 1rem;
  }

  .conversation-title-text {
    min-width: 0;
  }

  .conversation-title h3 {
    margin: 0;
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--color-text-heading);
  }

  .subject-display {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    color: var(--color-text-muted);
    font-size: 0.75rem;
    font-family: var(--font-body);
    transition: color 0.2s;
  }

  .subject-display:hover {
    color: var(--color-accent);
  }

  .subject-display .subject-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 300px;
  }

  .subject-edit {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.125rem;
  }

  .subject-input {
    padding: 0.2rem 0.5rem;
    font-size: 0.75rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text);
    font-family: var(--font-body);
    width: 200px;
    outline: none;
  }

  .subject-input:focus {
    border-color: var(--color-accent);
  }

  .subject-btn {
    padding: 0.2rem;
    border: none;
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: flex;
    align-items: center;
    transition: background 0.2s;
  }

  .subject-btn.save {
    background: var(--color-success);
    color: white;
  }

  .subject-btn.save:hover {
    filter: brightness(1.1);
  }

  .subject-btn.cancel {
    background: var(--color-surface);
    color: var(--color-text-muted);
  }

  .subject-btn.cancel:hover {
    background: var(--color-error-muted);
    color: var(--color-error);
  }

  .conversation-messages {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    background: var(--color-bg);
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
    color: var(--color-text-muted);
  }

  .empty-conversation :global(svg) {
    margin-bottom: 0.5rem;
  }

  .conversation-error {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: var(--color-error-muted);
    color: var(--color-error);
    font-size: 0.8rem;
    border-top: 1px solid rgba(239, 68, 68, 0.15);
  }

  .assistant-activity {
    align-self: flex-start;
    max-width: 75%;
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .thinking-indicator {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.75rem 1rem;
    background: var(--color-surface);
    border-radius: 1.25rem;
    border-bottom-left-radius: 0.25rem;
  }

  .thinking-indicator .dot {
    width: 8px;
    height: 8px;
    background: var(--color-text-muted);
    border-radius: 50%;
    animation: thinking-bounce 1.4s ease-in-out infinite;
  }

  .thinking-indicator .dot:nth-child(2) { animation-delay: 0.2s; }
  .thinking-indicator .dot:nth-child(3) { animation-delay: 0.4s; }

  @keyframes thinking-bounce {
    0%, 80%, 100% { opacity: 0.3; transform: scale(0.8); }
    40% { opacity: 1; transform: scale(1); }
  }

  .tool-calls-inline {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.25rem 0;
  }

  .tool-call-item {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.3rem 0.65rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 8px;
    font-size: 0.8125rem;
    color: var(--color-text-secondary);
    width: fit-content;
  }

  .tool-call-item.running {
    border-color: #fde68a;
    background: #fffbeb;
    color: #92400e;
  }

  .tool-call-item.completed {
    border-color: #bbf7d0;
    background: #f0fdf4;
    color: #059669;
  }

  .tool-call-item.failed {
    border-color: #fecaca;
    background: #fef2f2;
    color: #dc2626;
  }

  .tool-fn-name {
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-weight: 500;
    font-size: 0.75rem;
  }

  .tool-duration {
    font-size: 0.6875rem;
    opacity: 0.7;
  }

  .tool-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid #fde68a;
    border-top-color: #d97706;
    border-radius: 50%;
    animation: tool-spin 0.8s linear infinite;
  }

  @keyframes tool-spin {
    to { transform: rotate(360deg); }
  }

  .conversation-input {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-top: 1px solid var(--color-border);
    background: var(--color-bg-elevated);
    flex-shrink: 0;
  }

  .conversation-input input {
    flex: 1;
    padding: 0.625rem 1rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 999px;
    font-size: 0.875rem;
    color: var(--color-text);
    font-family: var(--font-body);
    outline: none;
    transition: border-color 0.2s, box-shadow 0.2s;
  }

  .conversation-input input::placeholder {
    color: var(--color-text-muted);
  }

  .conversation-input input:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px var(--color-accent-glow);
  }

  .btn-send-reply {
    padding: 0.625rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: 50%;
    cursor: pointer;
    transition: background 0.2s, box-shadow 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .btn-send-reply:hover:not(:disabled) {
    background: var(--color-accent-hover);
    box-shadow: 0 2px 10px rgba(212, 175, 55, 0.3);
  }

  .btn-send-reply:disabled {
    background: var(--color-surface);
    color: var(--color-text-muted);
    cursor: not-allowed;
  }

  @media (max-width: 640px) {
    .conversation-page {
      padding: 1rem;
      height: calc(100vh - 60px);
    }
  }
</style>
