<script lang="ts">
  import { Bell, Check, Trash2 } from 'lucide-svelte';
  import { notificationStore, unreadCount } from '$lib/stores/notifications';
  import NotificationItem from './NotificationItem.svelte';

  let isOpen = $state(false);
  let dropdownRef: HTMLDivElement | null = $state(null);

  function toggleDropdown() {
    isOpen = !isOpen;
  }

  function closeDropdown() {
    isOpen = false;
  }

  function handleClickOutside(e: MouseEvent) {
    if (dropdownRef && !dropdownRef.contains(e.target as Node)) {
      closeDropdown();
    }
  }

  async function handleMarkAllAsRead() {
    await notificationStore.markAllAsRead();
  }

  async function handleClearAll() {
    await notificationStore.clearAll();
  }

  // Handle click outside to close dropdown
  $effect(() => {
    if (isOpen) {
      document.addEventListener('click', handleClickOutside);
      return () => document.removeEventListener('click', handleClickOutside);
    }
  });

  // Get notifications from store
  const notifications = $derived($notificationStore);
  const count = $derived($unreadCount);
</script>

<div class="notification-bell" bind:this={dropdownRef}>
  <button
    class="bell-button"
    onclick={toggleDropdown}
    title="Notifications"
    aria-label="Notifications"
    aria-expanded={isOpen}
  >
    <Bell size={20} />
    {#if count > 0}
      <span class="badge">{count > 9 ? '9+' : count}</span>
    {/if}
  </button>

  {#if isOpen}
    <div class="dropdown">
      <div class="dropdown-header">
        <span class="dropdown-title">Notifications</span>
        <div class="header-actions">
          {#if count > 0}
            <button class="action-btn" onclick={handleMarkAllAsRead} title="Mark all as read">
              <Check size={14} />
              <span>Read all</span>
            </button>
          {/if}
          {#if notifications.length > 0}
            <button class="action-btn danger" onclick={handleClearAll} title="Clear all notifications">
              <Trash2 size={14} />
              <span>Clear all</span>
            </button>
          {/if}
        </div>
      </div>

      <div class="dropdown-content">
        {#if notifications.length === 0}
          <div class="empty-state">
            <Bell size={32} strokeWidth={1.5} />
            <p>No notifications</p>
          </div>
        {:else}
          {#each notifications as notification (notification.id)}
            <NotificationItem {notification} onNavigate={closeDropdown} />
          {/each}
        {/if}
      </div>

      <div class="dropdown-footer">
        <a href="/inbox" class="footer-link" onclick={closeDropdown}>View all messages</a>
        <span class="footer-divider">|</span>
        <a href="/friends" class="footer-link" onclick={closeDropdown}>Friends</a>
      </div>
    </div>
  {/if}
</div>

<style>
  .notification-bell {
    position: relative;
  }

  .bell-button {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.5rem;
    border-radius: 8px;
    color: #6b7280;
    background: transparent;
    border: none;
    cursor: pointer;
    transition: background 0.2s, color 0.2s;
  }

  .bell-button:hover {
    background: #f3f4f6;
    color: #6366f1;
  }

  .badge {
    position: absolute;
    top: 0;
    right: 0;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    background: #ef4444;
    color: white;
    font-size: 0.625rem;
    font-weight: 700;
    border-radius: 999px;
    display: flex;
    align-items: center;
    justify-content: center;
    transform: translate(25%, -25%);
  }

  .dropdown {
    position: absolute;
    top: calc(100% + 0.5rem);
    right: 0;
    width: 340px;
    max-height: 480px;
    background: white;
    border-radius: 12px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.15), 0 2px 10px rgba(0, 0, 0, 0.1);
    overflow: hidden;
    z-index: 1000;
    display: flex;
    flex-direction: column;
  }

  .dropdown-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem;
    border-bottom: 1px solid #e5e7eb;
    background: #fafafa;
  }

  .dropdown-title {
    font-size: 0.9375rem;
    font-weight: 600;
    color: #111827;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .action-btn {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.25rem 0.5rem;
    font-size: 0.75rem;
    color: #6366f1;
    background: transparent;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
  }

  .action-btn:hover {
    background: #f0f0ff;
  }

  .action-btn.danger {
    color: #6b7280;
  }

  .action-btn.danger:hover {
    background: #fee2e2;
    color: #ef4444;
  }

  .dropdown-content {
    flex: 1;
    overflow-y: auto;
    max-height: 360px;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 3rem 1rem;
    color: #9ca3af;
  }

  .empty-state p {
    margin-top: 0.75rem;
    font-size: 0.875rem;
  }

  .dropdown-footer {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-top: 1px solid #e5e7eb;
    background: #fafafa;
  }

  .footer-link {
    font-size: 0.8125rem;
    color: #6366f1;
    text-decoration: none;
    transition: color 0.15s;
  }

  .footer-link:hover {
    color: #4f46e5;
    text-decoration: underline;
  }

  .footer-divider {
    color: #d1d5db;
    font-size: 0.75rem;
  }

  @media (max-width: 480px) {
    .dropdown {
      width: calc(100vw - 2rem);
      right: -0.5rem;
    }
  }
</style>
