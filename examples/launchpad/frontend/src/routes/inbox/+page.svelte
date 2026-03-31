<script lang="ts">
  import { user } from '$lib/stores/auth';
  import {
    markAsRead,
    respondToFriendRequest,
  } from '$lib/stores/messaging';
  import {
    messagingStore,
    conversationsArray,
    inboxMessages,
    unreadCount as messagingUnreadCount,
    type Message,
    type Conversation
  } from '$lib/stores/messaging-store';
  import { goto, invalidateAll } from '$app/navigation';
  import {
    Inbox,
    Mail,
    Users,
    Bell,
    AlertCircle,
    CheckCircle,
    Clock,
    XCircle,
    Eye,
    Check,
    X,
    MessageCircle,
    Edit3
  } from 'lucide-svelte';
  import ComposeMessage from '$lib/components/ComposeMessage.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  // State for actions
  let processingId = $state<string | null>(null);
  let showCompose = $state(false);
  let actionError = $state<string | null>(null);
  let actionSuccess = $state<string | null>(null);

  // Tab state: 'all' | 'conversations'
  let activeTab = $state<'all' | 'conversations'>('conversations');

  // Get current user path for determining message ownership

  function handleMessageSent() {
    // Refresh messaging store to pick up the new message
    messagingStore.refresh();
    // Also invalidate for All Messages tab
    invalidateAll();
  }

  // Get message type icon
  function getMessageTypeIcon(messageType: string) {
    switch (messageType) {
      case 'relationship_request_received':
        return Users;
      case 'chat':
        return MessageCircle;
      case 'relationship_response_notification':
      case 'system_notification':
        return Bell;
      default:
        return Mail;
    }
  }

  // Get message type label
  function getMessageTypeLabel(messageType: string): string {
    switch (messageType) {
      case 'relationship_request_received':
        return 'Friend Request';
      case 'chat':
        return 'Message';
      case 'relationship_response_notification':
      case 'system_notification':
        return 'Notification';
      default:
        return 'Message';
    }
  }

  // Get status display
  function getStatusDisplay(status: string): { icon: typeof CheckCircle; color: string; label: string } {
    switch (status) {
      case 'delivered':
        return { icon: CheckCircle, color: 'text-green-600', label: 'New' };
      case 'pending':
        return { icon: Clock, color: 'text-yellow-600', label: 'Pending' };
      case 'error':
        return { icon: XCircle, color: 'text-red-600', label: 'Error' };
      case 'read':
        return { icon: CheckCircle, color: 'text-gray-400', label: 'Read' };
      case 'accepted':
        return { icon: CheckCircle, color: 'text-green-600', label: 'Accepted' };
      case 'declined':
        return { icon: XCircle, color: 'text-red-600', label: 'Declined' };
      default:
        return { icon: Clock, color: 'text-gray-600', label: status };
    }
  }

  // Format date
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
    } catch {
      return dateString;
    }
  }

  // Get message content preview
  function getMessagePreview(msg: Message | undefined): string {
    if (!msg) return '';
    const body = msg.properties.body || {};

    if (msg.properties.message_type === 'relationship_request_received') {
      return msg.properties.message || body.message || `${msg.properties.sender_display_name || msg.properties.sender_id || 'Someone'} wants to connect`;
    }
    if (msg.properties.message_type === 'chat') {
      return body.message_text || body.content || '';
    }
    if (msg.properties.message_type === 'relationship_response_notification' || msg.properties.message_type === 'system_notification') {
      return msg.properties.message || body.message || '';
    }
    return body.message || '';
  }

  // Get sender info
  function getSenderInfo(msg: Message): string {
    return msg.properties.sender_display_name || msg.properties.sender_id || 'Unknown';
  }

  // Handle accept friend request
  async function handleAccept(msg: Message) {
    processingId = msg.id;
    actionError = null;
    actionSuccess = null;
    const result = await respondToFriendRequest(msg, true);
    if (result.success) {
      actionSuccess = 'Friend request accepted!';
      await invalidateAll();
    } else {
      actionError = result.error || 'Failed to accept request';
    }
    processingId = null;
  }

  // Handle decline friend request
  async function handleDecline(msg: Message) {
    processingId = msg.id;
    actionError = null;
    actionSuccess = null;
    const result = await respondToFriendRequest(msg, false);
    if (result.success) {
      actionSuccess = 'Friend request declined';
      await invalidateAll();
    } else {
      actionError = result.error || 'Failed to decline request';
    }
    processingId = null;
  }

  // Handle mark as read
  async function handleMarkAsRead(msg: Message) {
    processingId = msg.id;
    await markAsRead(msg.path);
    await invalidateAll();
    processingId = null;
  }

  // Check if message is unread
  function isUnread(msg: Message): boolean {
    return !['read', 'accepted', 'declined'].includes(msg.properties.status);
  }

  // Check if message needs action
  function needsAction(msg: Message): boolean {
    return msg.properties.message_type === 'relationship_request_received' &&
           ['pending', 'delivered'].includes(msg.properties.status);
  }

  // Navigate to conversation
  function selectConversation(conv: Conversation) {
    goto(`/inbox/${conv.id}`);
  }
</script>

<svelte:head>
  <title>Inbox - Launchpad</title>
</svelte:head>

<div class="inbox-page">
  <div class="page-header">
    <div class="header-content">
      <Inbox size={32} />
      <div>
        <h1>Inbox</h1>
        <p class="subtitle">
          {#if $messagingUnreadCount > 0}
            {$messagingUnreadCount} unread message{$messagingUnreadCount !== 1 ? 's' : ''}
          {:else}
            All caught up!
          {/if}
        </p>
      </div>
    </div>
    <button class="btn-compose" onclick={() => showCompose = true}>
      <Edit3 size={16} />
      Compose
    </button>
  </div>

  {#if !$user}
    <div class="not-logged-in">
      <AlertCircle size={24} />
      <p>Please <a href="/auth/login">log in</a> to view your inbox.</p>
    </div>
  {:else}
    <!-- Tabs -->
    <div class="tabs">
      <button
        class="tab"
        class:active={activeTab === 'conversations'}
        onclick={() => { activeTab = 'conversations'; }}
      >
        <MessageCircle size={16} />
        Conversations
        {#if $messagingUnreadCount > 0}
          <span class="tab-badge">{$messagingUnreadCount}</span>
        {/if}
      </button>
      <button
        class="tab"
        class:active={activeTab === 'all'}
        onclick={() => activeTab = 'all'}
      >
        <Inbox size={16} />
        All Messages
        {#if data.unreadCount > 0}
          <span class="tab-badge">{data.unreadCount}</span>
        {/if}
      </button>
    </div>

    <!-- Conversations Tab -->
    {#if activeTab === 'conversations'}
      <!-- Conversation List -->
      {#if $conversationsArray.length === 0}
          <div class="empty-state">
            <MessageCircle size={48} />
            <h3>No conversations yet</h3>
            <p>When you message friends, your conversations will appear here.</p>
          </div>
        {:else}
          <div class="conversations-list">
            {#each $conversationsArray as conv (conv.id)}
              <button
                class="conversation-item"
                class:unread={conv.unreadCount > 0}
                onclick={() => selectConversation(conv)}
              >
                <div class="conv-avatar">
                  {conv.participantDisplayName.charAt(0).toUpperCase()}
                </div>
                <div class="conv-content">
                  <div class="conv-header">
                    <span class="conv-name">{conv.participantDisplayName}</span>
                    <span class="conv-time">{formatDate(conv.lastMessageAt)}</span>
                  </div>
                  <div class="conv-preview">
                    <span class="conv-message">{getMessagePreview(conv.lastMessage)}</span>
                    {#if conv.unreadCount > 0}
                      <span class="conv-badge">{conv.unreadCount}</span>
                    {/if}
                  </div>
                </div>
              </button>
            {/each}
          </div>
        {/if}
    {:else}
      <!-- All Messages Tab -->
      {#if actionError}
        <div class="action-error">
          <AlertCircle size={16} />
          {actionError}
        </div>
      {/if}
      {#if actionSuccess}
        <div class="action-success">
          <CheckCircle size={16} />
          {actionSuccess}
        </div>
      {/if}
      {#if $inboxMessages.length === 0}
        <div class="empty-state">
          <Inbox size={48} />
          <h3>Your inbox is empty</h3>
          <p>When you receive messages or friend requests, they'll appear here.</p>
        </div>
      {:else}
        <div class="messages-list">
      {#each $inboxMessages as msg, i (`${i}-${msg.id}`)}
        {@const unread = isUnread(msg)}
        {@const status = getStatusDisplay(msg.properties.status)}
        {@const TypeIcon = getMessageTypeIcon(msg.properties.message_type)}

        <div class="message-item" class:unread>
          <div class="message-indicator">
            {#if unread}
              <span class="unread-dot"></span>
            {:else}
              <span class="read-dot"></span>
            {/if}
          </div>

          <div class="message-icon">
            <TypeIcon size={20} />
          </div>

          <div class="message-content">
            <div class="message-header">
              <span class="message-type">{getMessageTypeLabel(msg.properties.message_type)}</span>
              <span class="message-from">from {getSenderInfo(msg)}</span>
              <span class="message-time">{formatDate(msg.properties.created_at)}</span>
            </div>

            <p class="message-preview">{getMessagePreview(msg)}</p>

            {#if msg.properties.status === 'error' && msg.properties.error}
              <p class="message-error">
                <AlertCircle size={14} />
                {msg.properties.error}
              </p>
            {/if}
          </div>

          <div class="message-actions">
            {#if needsAction(msg)}
              <button
                class="btn-accept"
                onclick={() => handleAccept(msg)}
                disabled={processingId === msg.id}
              >
                <Check size={14} />
                Accept
              </button>
              <button
                class="btn-decline"
                onclick={() => handleDecline(msg)}
                disabled={processingId === msg.id}
              >
                <X size={14} />
                Decline
              </button>
            {:else if msg.properties.message_type === 'chat' && unread}
              <button
                class="btn-read"
                onclick={() => handleMarkAsRead(msg)}
                disabled={processingId === msg.id}
              >
                <Eye size={14} />
                Mark Read
              </button>
            {:else if unread && msg.properties.message_type !== 'relationship_request_received'}
              <button
                class="btn-read"
                onclick={() => handleMarkAsRead(msg)}
                disabled={processingId === msg.id}
              >
                <Eye size={14} />
                Mark Read
              </button>
            {:else}
              {@const StatusIcon = status.icon}
              <span class="status-badge {status.color}">
                <StatusIcon size={12} />
                {status.label}
              </span>
            {/if}
          </div>
        </div>
      {/each}
        </div>
      {/if}
    {/if}
  {/if}
</div>

{#if showCompose}
  <ComposeMessage
    onClose={() => showCompose = false}
    onSent={handleMessageSent}
  />
{/if}

<style>
  .inbox-page {
    max-width: 800px;
    margin: 0 auto;
    padding: 2rem;
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

  .header-content :global(svg) {
    color: #6366f1;
    flex-shrink: 0;
    margin-top: 0.25rem;
  }

  .page-header h1 {
    font-size: 1.75rem;
    font-weight: 700;
    color: #1f2937;
    margin: 0;
  }

  .subtitle {
    color: #6b7280;
    margin: 0.25rem 0 0;
  }

  .btn-compose {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1rem;
    background: #6366f1;
    color: white;
    border: none;
    border-radius: 8px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-compose:hover {
    background: #4f46e5;
  }

  .not-logged-in {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    background: #fef3c7;
    color: #92400e;
    padding: 1rem 1.5rem;
    border-radius: 8px;
  }

  .not-logged-in a {
    color: #6366f1;
    text-decoration: underline;
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
    border-bottom: 1px solid #e5e7eb;
    padding-bottom: 0.5rem;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: #6b7280;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .tab:hover {
    background: #f3f4f6;
    color: #374151;
  }

  .tab.active {
    background: #e0e7ff;
    color: #6366f1;
  }

  .tab-badge {
    background: #ef4444;
    color: white;
    font-size: 0.625rem;
    font-weight: 700;
    padding: 0.125rem 0.375rem;
    border-radius: 999px;
    min-width: 18px;
    text-align: center;
  }

  /* Conversations List */
  .conversations-list {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .conversation-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.875rem 1rem;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
    width: 100%;
  }

  .conversation-item:hover {
    background: #f9fafb;
    border-color: #d1d5db;
  }

  .conversation-item.unread {
    background: #f0f9ff;
    border-color: #bae6fd;
  }

  .conv-avatar {
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
    flex-shrink: 0;
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
    margin-bottom: 0.25rem;
  }

  .conv-name {
    font-weight: 600;
    color: #1f2937;
    font-size: 0.875rem;
  }

  .conv-time {
    font-size: 0.75rem;
    color: #9ca3af;
    flex-shrink: 0;
  }

  .conv-preview {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .conv-message {
    color: #6b7280;
    font-size: 0.875rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }

  .conversation-item.unread .conv-message {
    color: #374151;
    font-weight: 500;
  }

  .conv-badge {
    background: #6366f1;
    color: white;
    font-size: 0.625rem;
    font-weight: 700;
    padding: 0.125rem 0.375rem;
    border-radius: 999px;
    min-width: 18px;
    text-align: center;
    flex-shrink: 0;
  }

  .empty-state {
    text-align: center;
    padding: 4rem 2rem;
    background: #f9fafb;
    border-radius: 12px;
  }

  .empty-state :global(svg) {
    color: #d1d5db;
    margin-bottom: 1rem;
  }

  .empty-state h3 {
    font-size: 1.125rem;
    font-weight: 600;
    color: #374151;
    margin: 0 0 0.5rem;
  }

  .empty-state p {
    color: #6b7280;
    margin: 0;
  }

  .messages-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .message-item {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 1rem;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    transition: background 0.2s;
  }

  .message-item:hover {
    background: #f9fafb;
  }

  .message-item.unread {
    background: #f0f9ff;
    border-color: #bae6fd;
  }

  .message-indicator {
    width: 8px;
    padding-top: 0.375rem;
  }

  .unread-dot {
    display: block;
    width: 8px;
    height: 8px;
    background: #6366f1;
    border-radius: 50%;
  }

  .read-dot {
    display: block;
    width: 8px;
    height: 8px;
    background: transparent;
  }

  .message-icon {
    padding: 0.5rem;
    background: #f3f4f6;
    border-radius: 8px;
    color: #6b7280;
  }

  .message-item.unread .message-icon {
    background: #e0e7ff;
    color: #6366f1;
  }

  .message-content {
    flex: 1;
    min-width: 0;
  }

  .message-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-bottom: 0.25rem;
  }

  .message-type {
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    color: #6366f1;
    background: #e0e7ff;
    padding: 0.125rem 0.5rem;
    border-radius: 4px;
  }

  .message-from {
    font-weight: 500;
    color: #1f2937;
    font-size: 0.875rem;
  }

  .message-time {
    font-size: 0.75rem;
    color: #9ca3af;
    margin-left: auto;
  }

  .message-preview {
    color: #4b5563;
    font-size: 0.875rem;
    margin: 0;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .message-error {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    color: #dc2626;
    font-size: 0.75rem;
    margin: 0.375rem 0 0;
  }

  .message-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }

  .btn-accept {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.375rem 0.75rem;
    background: #10b981;
    color: white;
    border: none;
    border-radius: 6px;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-accept:hover:not(:disabled) {
    background: #059669;
  }

  .btn-decline {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.375rem 0.75rem;
    background: white;
    color: #dc2626;
    border: 1px solid #fecaca;
    border-radius: 6px;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-decline:hover:not(:disabled) {
    background: #fef2f2;
  }

  .btn-read {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.375rem 0.75rem;
    background: #f3f4f6;
    color: #6b7280;
    border: 1px solid #e5e7eb;
    border-radius: 6px;
    font-size: 0.75rem;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-read:hover:not(:disabled) {
    background: #e5e7eb;
    color: #374151;
  }

  .btn-accept:disabled,
  .btn-decline:disabled,
  .btn-read:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .status-badge {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    padding: 0.25rem 0.5rem;
    background: #f3f4f6;
    border-radius: 4px;
  }

  .action-error,
  .action-success {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    font-size: 0.875rem;
    margin-bottom: 1rem;
  }

  .action-error {
    background: #fef2f2;
    color: #dc2626;
  }

  .action-success {
    background: #f0fdf4;
    color: #16a34a;
  }

  @media (max-width: 640px) {
    .inbox-page {
      padding: 1rem;
    }

    .page-header {
      flex-direction: column;
      gap: 1rem;
    }

    .btn-compose {
      width: 100%;
      justify-content: center;
    }

    .message-item {
      flex-wrap: wrap;
    }

    .message-actions {
      width: 100%;
      margin-top: 0.75rem;
      padding-top: 0.75rem;
      border-top: 1px solid #e5e7eb;
    }
  }
</style>
