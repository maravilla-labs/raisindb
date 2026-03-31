<script lang="ts">
  import { Bot, Plus, Trash2, MessageSquare, ChevronLeft } from 'lucide-svelte';
  import type { AIConversation } from '$lib/stores/ai-chat';

  interface Props {
    conversations: AIConversation[];
    activeConversationId: string | null;
    onSelect: (convId: string) => void;
    onNewChat: () => void;
    onDelete: (convId: string) => void;
    onClose: () => void;
  }

  let { conversations, activeConversationId, onSelect, onNewChat, onDelete, onClose }: Props = $props();

  // Sort conversations by updatedAt (most recently active first)
  const sortedConversations = $derived(
    [...conversations].sort((a, b) =>
      new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime()
    )
  );

  function formatDate(dateStr: string): string {
    const date = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

    if (diffDays === 0) {
      return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    } else if (diffDays === 1) {
      return 'Yesterday';
    } else if (diffDays < 7) {
      return date.toLocaleDateString([], { weekday: 'short' });
    } else {
      return date.toLocaleDateString([], { month: 'short', day: 'numeric' });
    }
  }

  function getAgentName(conv: AIConversation): string {
    const path = conv.agentRef?.['raisin:path'];
    if (path) {
      const name = path.split('/').pop() || 'Assistant';
      return name.replace(/-/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
    }
    return 'Assistant';
  }

  function handleDelete(e: Event, convId: string) {
    e.stopPropagation();
    if (confirm('Delete this conversation?')) {
      onDelete(convId);
    }
  }
</script>

<div class="conversation-list">
  <div class="list-header">
    <button class="back-button" onclick={onClose}>
      <ChevronLeft size={18} />
    </button>
    <span class="header-title">Conversations</span>
    <button class="new-chat-button" onclick={onNewChat} title="New chat">
      <Plus size={18} />
    </button>
  </div>

  <div class="conversations">
    {#if sortedConversations.length === 0}
      <div class="empty-state">
        <MessageSquare size={32} class="text-zinc-300" />
        <p>No conversations yet</p>
        <button class="start-chat-button" onclick={onNewChat}>
          Start a new chat
        </button>
      </div>
    {:else}
      {#each sortedConversations as conv (conv.id)}
        <div
          class="conversation-item"
          class:active={conv.id === activeConversationId}
          onclick={() => onSelect(conv.id)}
          onkeydown={(e) => e.key === 'Enter' && onSelect(conv.id)}
          role="button"
          tabindex="0"
        >
          <div class="conv-icon">
            <Bot size={18} />
          </div>
          <div class="conv-info">
            <span class="conv-title">{conv.title || getAgentName(conv)}</span>
            <span class="conv-meta">
              {getAgentName(conv)} &middot; {formatDate(conv.updatedAt)}
            </span>
          </div>
          <button
            class="delete-button"
            onclick={(e) => handleDelete(e, conv.id)}
            title="Delete conversation"
          >
            <Trash2 size={14} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .conversation-list {
    display: flex;
    flex-direction: column;
    background: #fafafa;
    min-height: 300px;
    max-height: 400px;
  }

  .list-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    background: white;
    border-bottom: 1px solid #e5e7eb;
  }

  .back-button {
    padding: 0.375rem;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: #6b7280;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .back-button:hover {
    background: #f3f4f6;
    color: #374151;
  }

  .header-title {
    flex: 1;
    font-weight: 600;
    font-size: 0.875rem;
    color: #1f2937;
  }

  .new-chat-button {
    padding: 0.375rem;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    border: none;
    border-radius: 6px;
    color: white;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.2s;
  }

  .new-chat-button:hover {
    transform: scale(1.05);
    box-shadow: 0 2px 8px rgba(139, 92, 246, 0.4);
  }

  .conversations {
    flex: 1;
    overflow-y: auto;
    padding: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 2rem;
    color: #9ca3af;
    text-align: center;
    gap: 0.75rem;
  }

  .empty-state p {
    margin: 0;
    font-size: 0.875rem;
  }

  .start-chat-button {
    margin-top: 0.5rem;
    padding: 0.5rem 1rem;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .start-chat-button:hover {
    transform: scale(1.02);
    box-shadow: 0 2px 8px rgba(139, 92, 246, 0.4);
  }

  .conversation-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
    width: 100%;
  }

  .conversation-item:hover {
    border-color: #d1d5db;
    background: #f9fafb;
  }

  .conversation-item.active {
    border-color: #8b5cf6;
    background: #faf5ff;
  }

  .conv-icon {
    width: 36px;
    height: 36px;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .conv-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .conv-title {
    font-weight: 500;
    font-size: 0.875rem;
    color: #1f2937;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .conv-meta {
    font-size: 0.75rem;
    color: #9ca3af;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .delete-button {
    padding: 0.375rem;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: #d1d5db;
    cursor: pointer;
    opacity: 0;
    transition: all 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .conversation-item:hover .delete-button {
    opacity: 1;
  }

  .delete-button:hover {
    background: #fee2e2;
    color: #ef4444;
  }

  :global(.text-zinc-300) {
    color: #d4d4d8;
  }
</style>
