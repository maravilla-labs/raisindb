<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { inbox } from '$lib/stores/inbox-store.svelte';
  import { getDatabase } from '$lib/raisin';
  import { goto } from '$app/navigation';
  import {
    Inbox,
    AlertCircle,
    MessageCircle,
    Edit3,
    Bot,
  } from 'lucide-svelte';
  import ComposeMessage from '$lib/components/ComposeMessage.svelte';

  let showCompose = $state(false);

  // Initialize inbox store when user is available
  $effect(() => {
    if ($user?.home) {
      getDatabase().then(db => inbox.init(db));
    }
    return () => inbox.destroy();
  });

  function formatDate(dateString?: string): string {
    if (!dateString) return '';
    try {
      const date = new Date(dateString);
      const now = new Date();
      const diffMs = now.getTime() - date.getTime();
      const diffMins = Math.floor(diffMs / 60000);
      const diffHours = Math.floor(diffMs / 3600000);
      const diffDays = Math.floor(diffMs / 86400000);
      if (diffMins < 1) return 'Just now';
      if (diffMins < 60) return `${diffMins}m ago`;
      if (diffHours < 24) return `${diffHours}h ago`;
      if (diffDays < 7) return `${diffDays}d ago`;
      return date.toLocaleDateString();
    } catch { return dateString; }
  }

  function selectConversation(convPath: string) {
    const name = convPath.split('/').pop() || convPath;
    goto(`/inbox/${name}`);
  }

  function getConversationDisplayName(conv: { agentRef?: string | { 'raisin:path'?: string }; lastMessage?: { sender_id: string }; participants?: string[]; conversationPath?: string }): string {
    if (conv.agentRef) {
      const ref = conv.agentRef;
      const refPath = typeof ref === 'string' ? ref : ref['raisin:path'] || '';
      const agentName = refPath.split('/').pop() || 'Agent';
      return agentName.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
    }
    if (conv.lastMessage?.sender_id) return conv.lastMessage.sender_id;
    if (conv.participants?.length) return conv.participants[0] || 'User';
    if (conv.conversationPath) return conv.conversationPath.split('/').pop() || 'Conversation';
    return 'Conversation';
  }

  function isAgentConversation(conv: { type: string; agentRef?: string }): boolean {
    return conv.type === 'ai_chat' || !!conv.agentRef;
  }

  function handleConversationCreated(convPath: string) {
    showCompose = false;
    const name = convPath.split('/').pop() || convPath;
    goto(`/inbox/${name}`);
  }
</script>

<svelte:head>
  <title>Inbox - Nachtkultur</title>
</svelte:head>

<div class="inbox-page">
  <div class="page-header">
    <div class="header-content">
      <div class="header-icon">
        <Inbox size={20} />
      </div>
      <div>
        <h1>Inbox</h1>
        <p class="subtitle">
          {#if inbox.unreadCount > 0}
            {inbox.unreadCount} unread message{inbox.unreadCount !== 1 ? 's' : ''}
          {:else}
            All caught up
          {/if}
        </p>
      </div>
    </div>
    <button class="btn-compose" onclick={() => showCompose = true}>
      <Edit3 size={14} />
      New Chat
    </button>
  </div>

  {#if !$user}
    <div class="not-logged-in">
      <AlertCircle size={18} />
      <p>Please <a href="/auth/login">sign in</a> to view your inbox.</p>
    </div>
  {:else if inbox.conversations.length === 0}
    <div class="empty-state">
      <MessageCircle size={40} />
      <h3>No conversations yet</h3>
      <p>Start a conversation with a friend or AI agent.</p>
    </div>
  {:else}
    <div class="conversations-list">
      {#each inbox.conversations as conv (conv.id)}
        {@const displayName = getConversationDisplayName(conv)}
        {@const isAgent = isAgentConversation(conv)}
        <button
          class="conversation-item"
          class:unread={(conv.unreadCount ?? 0) > 0}
          onclick={() => selectConversation(conv.conversationPath)}
        >
          <div class="conv-avatar" class:agent={isAgent}>
            {#if isAgent}
              <Bot size={18} />
            {:else}
              {displayName.charAt(0).toUpperCase()}
            {/if}
          </div>
          <div class="conv-content">
            <div class="conv-header">
              <span class="conv-name">{displayName}</span>
              <span class="conv-time">{formatDate(conv.updatedAt)}</span>
            </div>
            <div class="conv-preview">
              <span class="conv-message">{conv.lastMessage?.content ?? ''}</span>
              {#if (conv.unreadCount ?? 0) > 0}
                <span class="conv-badge">{conv.unreadCount}</span>
              {/if}
            </div>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</div>

{#if showCompose}
  <ComposeMessage
    onClose={() => showCompose = false}
    onConversationCreated={handleConversationCreated}
  />
{/if}

<style>
  .inbox-page {
    max-width: 800px;
    margin: 0 auto;
    padding: 2.5rem 2rem;
    animation: fadeInUp 0.4s ease both;
  }

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 2rem;
  }

  .header-content {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
  }

  .header-icon {
    width: 40px;
    height: 40px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-accent);
    flex-shrink: 0;
  }

  .page-header h1 {
    font-family: var(--font-display);
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0;
    letter-spacing: -0.02em;
  }

  .subtitle {
    color: var(--color-text-muted);
    margin: 0.25rem 0 0;
    font-size: 0.85rem;
  }

  .btn-compose {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 1rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-sm);
    font-weight: 600;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.2s;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    font-family: var(--font-body);
  }

  .btn-compose:hover {
    background: var(--color-accent-hover);
    box-shadow: 0 4px 16px rgba(212, 175, 55, 0.2);
  }

  .not-logged-in {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    background: var(--color-warning-muted);
    color: var(--color-warning);
    padding: 1rem 1.25rem;
    border-radius: var(--radius-md);
    border: 1px solid rgba(245, 158, 11, 0.15);
    font-size: 0.9rem;
  }

  .not-logged-in a {
    color: var(--color-accent);
  }

  .conversations-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .conversation-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.875rem 1rem;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
    width: 100%;
    font-family: var(--font-body);
  }

  .conversation-item:hover {
    background: var(--color-bg-card-hover);
    border-color: var(--color-border);
  }

  .conversation-item.unread {
    border-color: var(--color-border-accent);
    background: var(--color-accent-glow);
  }

  .conv-avatar {
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--color-accent), var(--color-rose));
    color: var(--color-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.85rem;
    font-family: var(--font-display);
    flex-shrink: 0;
  }

  .conv-avatar.agent {
    background: linear-gradient(135deg, #8b5cf6, #6366f1);
  }

  .conv-content {
    flex: 1;
    min-width: 0;
  }

  .conv-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.2rem;
  }

  .conv-name {
    font-weight: 600;
    color: var(--color-text-heading);
    font-size: 0.875rem;
  }

  .conv-time {
    font-size: 0.7rem;
    color: var(--color-text-muted);
    flex-shrink: 0;
  }

  .conv-preview {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .conv-message {
    color: var(--color-text-secondary);
    font-size: 0.825rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .conversation-item.unread .conv-message {
    color: var(--color-text);
  }

  .conv-badge {
    background: var(--color-accent);
    color: var(--color-bg);
    font-size: 0.6rem;
    font-weight: 700;
    padding: 0.1rem 0.375rem;
    border-radius: 999px;
    min-width: 16px;
    text-align: center;
    flex-shrink: 0;
  }

  .empty-state {
    text-align: center;
    padding: 4rem 2rem;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-lg);
  }

  .empty-state :global(svg) {
    color: var(--color-text-muted);
    margin-bottom: 1rem;
  }

  .empty-state h3 {
    font-family: var(--font-display);
    font-size: 1rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0 0 0.375rem;
  }

  .empty-state p {
    color: var(--color-text-muted);
    margin: 0;
    font-size: 0.85rem;
  }

  @media (max-width: 640px) {
    .inbox-page {
      padding: 1.5rem 1rem;
    }

    .page-header {
      flex-direction: column;
      gap: 1rem;
    }

    .btn-compose {
      width: 100%;
      justify-content: center;
    }
  }
</style>
