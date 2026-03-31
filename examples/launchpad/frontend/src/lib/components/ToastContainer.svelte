<script lang="ts">
  import { goto } from '$app/navigation';
  import { toastStore, type Toast } from '$lib/stores/toast';
  import { MessageSquare, UserPlus, Bell, CheckCircle, XCircle, X } from 'lucide-svelte';

  const toasts = $derived($toastStore);

  function getIcon(type: Toast['type']) {
    switch (type) {
      case 'message':
        return MessageSquare;
      case 'relationship_request':
        return UserPlus;
      case 'success':
        return CheckCircle;
      case 'error':
        return XCircle;
      default:
        return Bell;
    }
  }

  function getIconColor(type: Toast['type']): string {
    switch (type) {
      case 'message':
        return '#6366f1';
      case 'relationship_request':
        return '#10b981';
      case 'success':
        return '#22c55e';
      case 'error':
        return '#ef4444';
      default:
        return '#6b7280';
    }
  }

  function handleClick(toast: Toast) {
    if (toast.link) {
      // Extract conversation ID from link if it's a message
      if (toast.type === 'message') {
        const match = toast.link.match(/\/chats\/([^/]+)/);
        if (match) {
          goto(`/inbox/${match[1]}`);
        } else {
          goto('/inbox');
        }
      } else if (toast.type === 'relationship_request') {
        goto('/friends');
      }
    }
    toastStore.dismiss(toast.id);
  }

  function handleDismiss(e: Event, id: string) {
    e.stopPropagation();
    toastStore.dismiss(id);
  }
</script>

{#if toasts.length > 0}
  <div class="toast-container">
    {#each toasts as toast (toast.id)}
      {@const Icon = getIcon(toast.type)}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="toast"
        class:clickable={!!toast.link}
        onclick={() => handleClick(toast)}
        role={toast.link ? 'button' : 'alert'}
        tabindex={toast.link ? 0 : -1}
      >
        <div class="toast-icon" style="color: {getIconColor(toast.type)}">
          <Icon size={20} />
        </div>
        <div class="toast-content">
          <div class="toast-title">{toast.title}</div>
          {#if toast.body}
            <div class="toast-body">{toast.body}</div>
          {/if}
        </div>
        <button class="toast-close" onclick={(e) => handleDismiss(e, toast.id)}>
          <X size={16} />
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    z-index: 9999;
    max-width: 360px;
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 0.875rem 1rem;
    background: white;
    border-radius: 12px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.15), 0 2px 10px rgba(0, 0, 0, 0.1);
    animation: slideIn 0.3s ease-out;
    pointer-events: auto;
    position: relative;
  }

  .toast.clickable {
    cursor: pointer;
    transition: transform 0.15s, box-shadow 0.15s;
  }

  .toast.clickable:hover {
    transform: translateY(-2px);
    box-shadow: 0 12px 45px rgba(0, 0, 0, 0.18), 0 4px 12px rgba(0, 0, 0, 0.12);
  }

  @keyframes slideIn {
    from {
      opacity: 0;
      transform: translateX(100%);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  .toast-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: #f3f4f6;
  }

  .toast-content {
    flex: 1;
    min-width: 0;
    padding-right: 1.5rem;
  }

  .toast-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: #111827;
    line-height: 1.3;
  }

  .toast-body {
    font-size: 0.8125rem;
    color: #6b7280;
    margin-top: 0.125rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toast-close {
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
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .toast:hover .toast-close {
    opacity: 1;
  }

  .toast-close:hover {
    background: #f3f4f6;
    color: #374151;
  }

  @media (max-width: 480px) {
    .toast-container {
      left: 1rem;
      right: 1rem;
      bottom: 1rem;
      max-width: none;
    }
  }
</style>
