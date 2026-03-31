<script lang="ts">
  import { goto } from '$app/navigation';
  import { toastStore, type Toast } from '$lib/stores/toast.svelte';
  import { X, Bell, CheckCircle, AlertTriangle, AlertCircle, Info } from 'lucide-svelte';

  function handleClick(toast: Toast) {
    if (toast.link) {
      toastStore.remove(toast.id);
      goto(toast.link);
    }
  }

  function handleDismiss(e: Event, id: string) {
    e.stopPropagation();
    toastStore.remove(id);
  }

  function getIcon(type: Toast['type']) {
    switch (type) {
      case 'success':
        return CheckCircle;
      case 'warning':
        return AlertTriangle;
      case 'error':
        return AlertCircle;
      default:
        return Bell;
    }
  }
</script>

<div class="toast-container" aria-live="polite" aria-atomic="true">
  {#each toastStore.toasts as toast (toast.id)}
    {@const IconComponent = getIcon(toast.type)}
    {#if toast.link}
      <button
        class="toast toast-{toast.type} clickable"
        onclick={() => handleClick(toast)}
      >
        <div class="toast-icon">
          <IconComponent size={20} />
        </div>
        <div class="toast-content">
          <div class="toast-title">{toast.title}</div>
          {#if toast.body}
            <div class="toast-body">{toast.body}</div>
          {/if}
        </div>
        <span
          class="toast-close"
          onclick={(e) => handleDismiss(e, toast.id)}
          role="button"
          tabindex="0"
          onkeydown={(e) => e.key === 'Enter' && handleDismiss(e, toast.id)}
          aria-label="Dismiss notification"
        >
          <X size={16} />
        </span>
      </button>
    {:else}
      <div class="toast toast-{toast.type}" role="alert">
        <div class="toast-icon">
          <IconComponent size={20} />
        </div>
        <div class="toast-content">
          <div class="toast-title">{toast.title}</div>
          {#if toast.body}
            <div class="toast-body">{toast.body}</div>
          {/if}
        </div>
        <button
          class="toast-close"
          onclick={(e) => handleDismiss(e, toast.id)}
          aria-label="Dismiss notification"
        >
          <X size={16} />
        </button>
      </div>
    {/if}
  {/each}
</div>

<style>
  .toast-container {
    position: fixed;
    top: 80px;
    right: 20px;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 400px;
    width: 100%;
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 1rem;
    background: white;
    border-radius: 0.5rem;
    box-shadow:
      0 4px 6px -1px rgb(0 0 0 / 0.1),
      0 2px 4px -2px rgb(0 0 0 / 0.1);
    border-left: 4px solid #6366f1;
    border-top: none;
    border-right: none;
    border-bottom: none;
    pointer-events: auto;
    animation: slideIn 0.3s ease-out;
    text-align: left;
    width: 100%;
    font-family: inherit;
    font-size: inherit;
  }

  .toast.clickable {
    cursor: pointer;
    transition: transform 0.2s, box-shadow 0.2s;
  }

  .toast.clickable:hover {
    transform: translateX(-4px);
    box-shadow:
      0 10px 15px -3px rgb(0 0 0 / 0.1),
      0 4px 6px -4px rgb(0 0 0 / 0.1);
  }

  .toast-success {
    border-left-color: #22c55e;
  }

  .toast-warning {
    border-left-color: #f59e0b;
  }

  .toast-error {
    border-left-color: #ef4444;
  }

  .toast-info {
    border-left-color: #6366f1;
  }

  .toast-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #6366f1;
  }

  .toast-success .toast-icon {
    color: #22c55e;
  }

  .toast-warning .toast-icon {
    color: #f59e0b;
  }

  .toast-error .toast-icon {
    color: #ef4444;
  }

  .toast-content {
    flex: 1;
    min-width: 0;
  }

  .toast-title {
    font-weight: 600;
    color: #1f2937;
    font-size: 0.875rem;
    line-height: 1.25;
  }

  .toast-body {
    color: #6b7280;
    font-size: 0.8125rem;
    margin-top: 0.25rem;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .toast-close {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: #9ca3af;
    cursor: pointer;
    border-radius: 4px;
    transition: background 0.2s, color 0.2s;
  }

  .toast-close:hover {
    background: #f3f4f6;
    color: #374151;
  }

  @keyframes slideIn {
    from {
      transform: translateX(100%);
      opacity: 0;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }

  @media (max-width: 480px) {
    .toast-container {
      left: 10px;
      right: 10px;
      max-width: none;
    }
  }
</style>
