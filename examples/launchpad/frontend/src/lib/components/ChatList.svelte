<script lang="ts">
  import { X, MessageCircle, Search } from 'lucide-svelte';
  import { chatStore } from '$lib/stores/chat';
  import { conversationsArray, messagingLoading, type Conversation } from '$lib/stores/messaging-store';
  import { presenceStore } from '$lib/stores/presence';

  let searchQuery = $state('');

  const filteredConversations = $derived(
    $conversationsArray.filter(c =>
      c.participantDisplayName.toLowerCase().includes(searchQuery.toLowerCase())
    )
  );

  function formatTime(dateStr: string): string {
    if (!dateStr) return '';
    const date = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMins / 60);
    const diffDays = Math.floor(diffHours / 24);

    if (diffMins < 1) return 'now';
    if (diffMins < 60) return `${diffMins}m`;
    if (diffHours < 24) return `${diffHours}h`;
    if (diffDays < 7) return `${diffDays}d`;

    return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  }

  function getPreviewText(conv: Conversation): string {
    const content = conv.lastMessage?.properties.body?.content ||
                   conv.lastMessage?.properties.body?.message || '';
    return content.length > 40 ? content.substring(0, 40) + '...' : content;
  }

  function handleConversationClick(conv: Conversation) {
    chatStore.openConversation(conv.id);
  }

  function handleClose() {
    chatStore.closeList();
  }
</script>

<div class="chat-list">
  <!-- Header -->
  <div class="chat-list-header">
    <div class="header-title">
      <MessageCircle size={20} />
      <span>Messages</span>
    </div>
    <button class="close-button" onclick={handleClose}>
      <X size={20} />
    </button>
  </div>

  <!-- Search -->
  <div class="search-container">
    <div class="search-input-wrapper">
      <Search size={16} />
      <input
        type="text"
        bind:value={searchQuery}
        placeholder="Search conversations..."
      />
    </div>
  </div>

  <!-- Conversations list -->
  <div class="conversations-container">
    {#if $messagingLoading}
      <div class="loading-state">
        <div class="spinner"></div>
      </div>
    {:else if filteredConversations.length === 0}
      <div class="empty-state">
        <MessageCircle size={40} />
        <span>
          {searchQuery ? 'No conversations found' : 'No conversations yet'}
        </span>
      </div>
    {:else}
      {#each filteredConversations as conv (conv.id)}
        <button
          class="conversation-item"
          class:unread={conv.unreadCount > 0}
          onclick={() => handleConversationClick(conv)}
        >
          <!-- Avatar -->
          <div class="avatar">
            {conv.participantDisplayName.charAt(0).toUpperCase()}
            <span
              class="online-indicator"
              class:online={presenceStore.isOnline(conv.participantId)}
              class:offline={!presenceStore.isOnline(conv.participantId)}
            ></span>
          </div>

          <!-- Content -->
          <div class="conversation-content">
            <div class="conversation-header">
              <span class="participant-name">
                {conv.participantDisplayName}
              </span>
              <span class="message-time">
                {formatTime(conv.lastMessageAt)}
              </span>
            </div>
            <div class="conversation-preview">
              <span class="preview-text">
                {getPreviewText(conv)}
              </span>
              {#if conv.unreadCount > 0}
                <span class="unread-badge">
                  {conv.unreadCount}
                </span>
              {/if}
            </div>
          </div>
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  .chat-list {
    width: 320px;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px 12px 0 0;
    box-shadow: 0 -4px 20px rgba(0, 0, 0, 0.15);
    display: flex;
    flex-direction: column;
    max-height: 450px;
    animation: slideUp 0.2s ease-out;
  }

  @keyframes slideUp {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .chat-list-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.875rem 1rem;
    background: #f9fafb;
    border-bottom: 1px solid #e5e7eb;
    border-radius: 12px 12px 0 0;
  }

  .header-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 600;
    color: #1f2937;
  }

  .header-title :global(svg) {
    color: #6366f1;
  }

  .close-button {
    padding: 0.375rem;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: #6b7280;
    cursor: pointer;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .close-button:hover {
    background: #e5e7eb;
    color: #374151;
  }

  .search-container {
    padding: 0.75rem;
    border-bottom: 1px solid #f3f4f6;
  }

  .search-input-wrapper {
    position: relative;
    display: flex;
    align-items: center;
  }

  .search-input-wrapper :global(svg) {
    position: absolute;
    left: 0.75rem;
    color: #9ca3af;
  }

  .search-input-wrapper input {
    width: 100%;
    padding: 0.5rem 0.75rem 0.5rem 2.25rem;
    font-size: 0.875rem;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    outline: none;
    transition: all 0.2s;
  }

  .search-input-wrapper input:focus {
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .conversations-container {
    flex: 1;
    overflow-y: auto;
  }

  .loading-state {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid #e5e7eb;
    border-top-color: #6366f1;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    color: #9ca3af;
    gap: 0.5rem;
  }

  .empty-state span {
    font-size: 0.875rem;
  }

  .conversation-item {
    width: 100%;
    padding: 0.875rem 1rem;
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    background: white;
    border: none;
    border-bottom: 1px solid #f3f4f6;
    cursor: pointer;
    transition: background 0.2s;
    text-align: left;
  }

  .conversation-item:hover {
    background: #f9fafb;
  }

  .conversation-item.unread {
    background: #f0f9ff;
  }

  .conversation-item:last-child {
    border-bottom: none;
  }

  .avatar {
    position: relative;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: linear-gradient(135deg, #6366f1, #8b5cf6);
    color: white;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: 0.875rem;
    flex-shrink: 0;
  }

  .online-indicator {
    position: absolute;
    bottom: 0;
    right: 0;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid white;
  }

  .online-indicator.online {
    background: #22c55e;
  }

  .online-indicator.offline {
    background: #9ca3af;
  }

  .conversation-content {
    flex: 1;
    min-width: 0;
  }

  .conversation-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .participant-name {
    font-weight: 600;
    font-size: 0.875rem;
    color: #1f2937;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .message-time {
    font-size: 0.75rem;
    color: #9ca3af;
    flex-shrink: 0;
  }

  .conversation-preview {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .preview-text {
    font-size: 0.8125rem;
    color: #6b7280;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .conversation-item.unread .preview-text {
    color: #374151;
    font-weight: 500;
  }

  .unread-badge {
    background: #6366f1;
    color: white;
    font-size: 0.6875rem;
    font-weight: 700;
    padding: 0.125rem 0.375rem;
    border-radius: 999px;
    min-width: 18px;
    text-align: center;
    flex-shrink: 0;
  }
</style>
