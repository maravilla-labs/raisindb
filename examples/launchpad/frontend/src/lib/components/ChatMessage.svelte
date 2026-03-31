<script lang="ts">
  import type { Message } from '$lib/stores/messaging';
  import { Check, Clock } from 'lucide-svelte';

  interface Props {
    message: Message;
    isMine: boolean;
    senderDisplayName?: string;
  }

  let { message, isMine, senderDisplayName = 'User' }: Props = $props();

  // Format timestamp to relative or time format
  function formatTime(dateStr: string | undefined): string {
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

  const content = $derived(
    message.properties.body?.message_text ||
    message.properties.body?.content ||
    message.properties.body?.message ||
    ''
  );
  const time = $derived(formatTime(message.properties.created_at));
  const senderName = $derived(senderDisplayName);
  const rawStatus = $derived(message.properties.status);
  const readBy = $derived(message.properties.read_by || []);

  // Determine display status: if read_by has entries, show as read
  const status = $derived(readBy.length > 0 ? 'read' : rawStatus);
</script>

<div class="chat-message" class:mine={isMine}>
  <div class="sender-name">{isMine ? 'Me' : senderName}</div>
  <div class="bubble">
    {content}
  </div>
  <div class="timestamp">
    {time}
    {#if isMine}
      <span class="status-icon" class:pending={status === 'pending'} class:sent={status === 'sent'} class:delivered={status === 'delivered'} class:read={status === 'read'}>
        {#if status === 'pending'}
          <Clock size={12} />
        {:else if status === 'sent'}
          <Check size={12} />
        {:else if status === 'delivered' || status === 'read'}
          <span class="double-check">
            <Check size={12} />
            <Check size={12} />
          </span>
        {/if}
      </span>
    {/if}
  </div>
</div>

<style>
  .chat-message {
    display: flex;
    flex-direction: column;
    max-width: 75%;
    margin-bottom: 0.75rem;
    animation: slideIn 0.3s ease-out;
  }

  @keyframes slideIn {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .chat-message.mine {
    align-self: flex-end;
    align-items: flex-end;
    animation-name: slideInRight;
  }

  @keyframes slideInRight {
    from {
      opacity: 0;
      transform: translateX(10px);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  .chat-message:not(.mine) {
    align-self: flex-start;
    align-items: flex-start;
    animation-name: slideInLeft;
  }

  @keyframes slideInLeft {
    from {
      opacity: 0;
      transform: translateX(-10px);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  .sender-name {
    font-size: 0.75rem;
    color: #6b7280;
    margin-bottom: 0.25rem;
    padding: 0 0.5rem;
  }

  .chat-message.mine .sender-name {
    text-align: right;
  }

  .bubble {
    padding: 0.625rem 1rem;
    border-radius: 1.25rem;
    font-size: 0.9375rem;
    line-height: 1.4;
    word-wrap: break-word;
    white-space: pre-wrap;
  }

  .chat-message.mine .bubble {
    background: linear-gradient(135deg, #6366f1, #4f46e5);
    color: white;
    border-bottom-right-radius: 0.25rem;
  }

  .chat-message:not(.mine) .bubble {
    background: #f3f4f6;
    color: #1f2937;
    border-bottom-left-radius: 0.25rem;
  }

  .timestamp {
    font-size: 0.6875rem;
    color: #9ca3af;
    margin-top: 0.25rem;
    padding: 0 0.5rem;
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .status-icon {
    display: inline-flex;
    align-items: center;
    transition: color 0.3s ease;
  }

  .status-icon.pending {
    color: #9ca3af;
  }

  .status-icon.sent {
    color: #9ca3af;
  }

  .status-icon.delivered {
    color: #6366f1;
  }

  .status-icon.read {
    color: #3b82f6;
  }

  .double-check {
    display: inline-flex;
    align-items: center;
  }

  .double-check :global(svg:last-child) {
    margin-left: -6px;
  }
</style>
