<script lang="ts">
  import { MessageSquare, UserPlus, Bell, Trash2 } from 'lucide-svelte';
  import { goto } from '$app/navigation';
  import type { Notification } from '$lib/stores/notifications';
  import { notificationStore } from '$lib/stores/notifications';

  interface Props {
    notification: Notification;
    onNavigate?: () => void;
  }

  let { notification, onNavigate }: Props = $props();

  const isUnread = $derived(!notification.properties.read);

  const iconColor = $derived(() => {
    switch (notification.properties.type) {
      case 'message':
        return '#6366f1';
      case 'relationship_request':
        return '#10b981';
      default:
        return '#6b7280';
    }
  });

  function formatTimeAgo(path: string, createdAt?: string): string {
    if (createdAt) {
      const timestamp = Date.parse(createdAt);
      if (!Number.isNaN(timestamp)) {
        const now = Date.now();
        const diffMs = now - timestamp;
        const diffMins = Math.floor(diffMs / 60000);
        const diffHours = Math.floor(diffMs / 3600000);
        const diffDays = Math.floor(diffMs / 86400000);

        if (diffMins < 1) return 'just now';
        if (diffMins < 60) return `${diffMins}m ago`;
        if (diffHours < 24) return `${diffHours}h ago`;
        if (diffDays < 7) return `${diffDays}d ago`;
        return new Date(timestamp).toLocaleDateString();
      }
    }

    // Extract timestamp from notification name (notif-{timestamp})
    const name = path.split('/').pop() || '';
    const match = name.match(/notif-(\d+)/);
    if (!match) return '';

    const timestamp = parseInt(match[1], 10);
    const now = Date.now();
    const diffMs = now - timestamp;
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMs / 3600000);
    const diffDays = Math.floor(diffMs / 86400000);

    if (diffMins < 1) return 'just now';
    if (diffMins < 60) return `${diffMins}m ago`;
    if (diffHours < 24) return `${diffHours}h ago`;
    if (diffDays < 7) return `${diffDays}d ago`;
    return new Date(timestamp).toLocaleDateString();
  }

  async function handleClick() {
    // Mark as read
    await notificationStore.markAsRead(notification.id);

    // Navigate based on notification type
    if (notification.properties.type === 'message' && notification.properties.link) {
      // Extract conversation ID from link
      // Link format: /users/{userId}/inbox/chats/{conversationId}
      const match = notification.properties.link.match(/\/chats\/([^/]+)/);
      if (match) {
        goto(`/inbox/${match[1]}`);
      } else {
        goto('/inbox');
      }
    } else if (notification.properties.type === 'relationship_request') {
      goto('/friends');
    }

    // Close the dropdown
    onNavigate?.();
  }

  async function handleDelete(e: Event) {
    e.stopPropagation();
    await notificationStore.delete(notification.id);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="notification-item"
  class:unread={isUnread}
  onclick={handleClick}
  role="button"
  tabindex="0"
>
  <div class="icon" style="color: {iconColor()}">
    {#if notification.properties.type === 'message'}
      <MessageSquare size={20} />
    {:else if notification.properties.type === 'relationship_request'}
      <UserPlus size={20} />
    {:else}
      <Bell size={20} />
    {/if}
  </div>

  <div class="content">
    <div class="title">{notification.properties.title}</div>
    {#if notification.properties.body}
      <div class="body">{notification.properties.body}</div>
    {/if}
    <div class="time">{formatTimeAgo(notification.path, notification.properties.data?.created_at)}</div>
  </div>

  <button class="delete-btn" onclick={handleDelete} title="Delete notification">
    <Trash2 size={14} />
  </button>

  {#if isUnread}
    <div class="unread-dot"></div>
  {/if}
</div>

<style>
  .notification-item {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    cursor: pointer;
    transition: background 0.15s;
    position: relative;
  }

  .notification-item:hover {
    background: #f3f4f6;
  }

  .notification-item.unread {
    background: #f0f9ff;
  }

  .notification-item.unread:hover {
    background: #e0f2fe;
  }

  .icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: #f3f4f6;
  }

  .content {
    flex: 1;
    min-width: 0;
  }

  .title {
    font-size: 0.875rem;
    font-weight: 500;
    color: #111827;
    line-height: 1.25;
  }

  .notification-item.unread .title {
    font-weight: 600;
  }

  .body {
    font-size: 0.8125rem;
    color: #6b7280;
    margin-top: 0.125rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .time {
    font-size: 0.75rem;
    color: #9ca3af;
    margin-top: 0.25rem;
  }

  .delete-btn {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    padding: 0.25rem;
    background: transparent;
    border: none;
    color: #9ca3af;
    cursor: pointer;
    border-radius: 4px;
    opacity: 0;
    transition: opacity 0.15s, background 0.15s, color 0.15s;
  }

  .notification-item:hover .delete-btn {
    opacity: 1;
  }

  .delete-btn:hover {
    background: #fee2e2;
    color: #ef4444;
  }

  .unread-dot {
    position: absolute;
    top: 50%;
    right: 0.75rem;
    transform: translateY(-50%);
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #6366f1;
  }

  .notification-item:hover .unread-dot {
    display: none;
  }
</style>
