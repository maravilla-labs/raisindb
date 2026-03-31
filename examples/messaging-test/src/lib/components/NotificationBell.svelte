<script lang="ts">
  import { goto } from '$app/navigation';
  import { notificationStore } from '$lib/stores/notifications.svelte';
  import { Bell, X, Check, ExternalLink } from 'lucide-svelte';

  let isOpen = $state(false);

  function toggleDropdown() {
    isOpen = !isOpen;
  }

  function closeDropdown() {
    isOpen = false;
  }

  function handleNotificationClick(link?: string) {
    if (link) {
      closeDropdown();
      goto(link);
    }
  }

  function handleMarkAllRead() {
    notificationStore.markAllAsRead();
  }

  function formatTime(dateStr?: string): string {
    if (!dateStr) return '';
    const date = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMins / 60);
    const diffDays = Math.floor(diffHours / 24);

    if (diffMins < 1) return 'Just now';
    if (diffMins < 60) return `${diffMins}m ago`;
    if (diffHours < 24) return `${diffHours}h ago`;
    if (diffDays < 7) return `${diffDays}d ago`;
    return date.toLocaleDateString();
  }
</script>

<div class="notification-bell">
  <button
    class="bell-button"
    onclick={toggleDropdown}
    aria-label="Notifications"
    aria-expanded={isOpen}
  >
    <Bell size={20} />
    {#if notificationStore.unreadCount > 0}
      <span class="badge">{notificationStore.unreadCount > 9 ? '9+' : notificationStore.unreadCount}</span>
    {/if}
  </button>

  {#if isOpen}
    <div class="dropdown-backdrop" onclick={closeDropdown} onkeydown={(e) => e.key === 'Escape' && closeDropdown()} role="button" tabindex="-1" aria-label="Close notifications"></div>
    <div class="dropdown" role="menu">
      <div class="dropdown-header">
        <h3>Notifications</h3>
        {#if notificationStore.unreadCount > 0}
          <button class="mark-all-read" onclick={handleMarkAllRead}>
            <Check size={14} />
            Mark all read
          </button>
        {/if}
      </div>

      <div class="dropdown-content">
        {#if notificationStore.notifications.length === 0}
          <div class="empty-state">
            <Bell size={32} strokeWidth={1.5} />
            <p>No notifications yet</p>
          </div>
        {:else}
          {#each notificationStore.notifications as notification (notification.id)}
            <button
              class="notification-item"
              class:unread={!notification.properties.read}
              onclick={() => handleNotificationClick(notification.properties.link)}
              role="menuitem"
            >
              <div class="notification-dot" class:visible={!notification.properties.read}></div>
              <div class="notification-content">
                <div class="notification-title">{notification.properties.title}</div>
                {#if notification.properties.body}
                  <div class="notification-body">{notification.properties.body}</div>
                {/if}
                <div class="notification-time">{formatTime(notification.properties.created_at)}</div>
              </div>
              {#if notification.properties.link}
                <ExternalLink size={14} class="link-icon" />
              {/if}
            </button>
          {/each}
        {/if}
      </div>

      <div class="dropdown-footer">
        <a href="/notifications" onclick={closeDropdown}>View all notifications</a>
      </div>
    </div>
  {/if}
</div>

<style>
  .notification-bell {
    position: relative;
  }

  .bell-button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    border: none;
    background: transparent;
    color: #4b5563;
    cursor: pointer;
    border-radius: 8px;
    transition: background 0.2s, color 0.2s;
    position: relative;
  }

  .bell-button:hover {
    background: #f3f4f6;
    color: #6366f1;
  }

  .badge {
    position: absolute;
    top: 4px;
    right: 4px;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    background: #ef4444;
    color: white;
    font-size: 0.6875rem;
    font-weight: 600;
    border-radius: 9px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dropdown-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10;
  }

  .dropdown {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    width: 360px;
    max-height: 480px;
    background: white;
    border-radius: 12px;
    box-shadow:
      0 20px 25px -5px rgb(0 0 0 / 0.1),
      0 8px 10px -6px rgb(0 0 0 / 0.1);
    border: 1px solid #e5e7eb;
    z-index: 20;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dropdown-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem;
    border-bottom: 1px solid #e5e7eb;
  }

  .dropdown-header h3 {
    font-size: 0.9375rem;
    font-weight: 600;
    color: #1f2937;
    margin: 0;
  }

  .mark-all-read {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.375rem 0.75rem;
    font-size: 0.75rem;
    color: #6366f1;
    background: #eef2ff;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.2s;
  }

  .mark-all-read:hover {
    background: #e0e7ff;
  }

  .dropdown-content {
    flex: 1;
    overflow-y: auto;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 3rem 1rem;
    color: #9ca3af;
    gap: 0.75rem;
  }

  .empty-state p {
    font-size: 0.875rem;
    margin: 0;
  }

  .notification-item {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    width: 100%;
    padding: 0.875rem 1rem;
    background: transparent;
    border: none;
    border-bottom: 1px solid #f3f4f6;
    cursor: pointer;
    text-align: left;
    transition: background 0.2s;
  }

  .notification-item:hover {
    background: #f9fafb;
  }

  .notification-item.unread {
    background: #fef3c7;
  }

  .notification-item.unread:hover {
    background: #fde68a;
  }

  .notification-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #6366f1;
    flex-shrink: 0;
    margin-top: 6px;
    opacity: 0;
  }

  .notification-dot.visible {
    opacity: 1;
  }

  .notification-content {
    flex: 1;
    min-width: 0;
  }

  .notification-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: #1f2937;
    line-height: 1.3;
  }

  .notification-body {
    font-size: 0.75rem;
    color: #6b7280;
    margin-top: 0.25rem;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .notification-time {
    font-size: 0.6875rem;
    color: #9ca3af;
    margin-top: 0.375rem;
  }

  .notification-item :global(.link-icon) {
    color: #9ca3af;
    flex-shrink: 0;
    margin-top: 4px;
  }

  .dropdown-footer {
    padding: 0.75rem 1rem;
    border-top: 1px solid #e5e7eb;
    text-align: center;
  }

  .dropdown-footer a {
    font-size: 0.8125rem;
    color: #6366f1;
    text-decoration: none;
    font-weight: 500;
  }

  .dropdown-footer a:hover {
    text-decoration: underline;
  }

  @media (max-width: 480px) {
    .dropdown {
      position: fixed;
      top: 60px;
      right: 10px;
      left: 10px;
      width: auto;
    }
  }
</style>
