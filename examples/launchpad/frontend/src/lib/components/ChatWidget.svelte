<script lang="ts">
  import { page } from '$app/stores';
  import { MessageCircle } from 'lucide-svelte';
  import { user } from '$lib/stores/auth';
  import {
    chatStore,
    openConversations,
    minimizedConversations,
    isListOpen
  } from '$lib/stores/chat';
  import {
    messagingStore,
    conversations,
    messages,
    unreadCount
  } from '$lib/stores/messaging-store';
  import ChatList from './ChatList.svelte';
  import ChatPopup from './ChatPopup.svelte';

  const currentUser = $derived($user);
  const isLoggedIn = $derived(!!currentUser?.home);
  // Hide chat widget on inbox page (it has its own chat UI)
  const isOnInboxPage = $derived($page.url.pathname.startsWith('/inbox'));

  // Get conversation data for open conversations (reactive)
  // Data comes from messagingStore, UI state from chatStore
  const openConversationData = $derived(
    $openConversations.map(id => ({
      id,
      conversation: $conversations.get(id),
      isMinimized: $minimizedConversations.includes(id)
    })).filter(c => c.conversation)
  );

  // No polling needed - messagingStore handles real-time updates via WebSocket

  function handleIconClick() {
    chatStore.toggleList();
  }
</script>

{#if isLoggedIn && !isOnInboxPage}
  <div class="chat-widget-container">
    <!-- Open conversation popups (from left to right, newest on right) -->
    {#each openConversationData as { id, conversation, isMinimized } (id)}
      {#if conversation}
        <ChatPopup
          conversationId={id}
          {isMinimized}
        />
      {/if}
    {/each}

    <!-- Conversation list dropdown -->
    {#if $isListOpen}
      <ChatList />
    {/if}

    <!-- Chat icon button -->
    <button
      onclick={handleIconClick}
      class="chat-icon-button"
      title="Messages"
    >
      <MessageCircle size={24} />

      <!-- Unread badge -->
      {#if $unreadCount > 0}
        <span class="unread-badge">
          {$unreadCount > 99 ? '99+' : $unreadCount}
        </span>
      {/if}
    </button>
  </div>
{/if}

<style>
  .chat-widget-container {
    position: fixed;
    bottom: 1rem;
    right: 1rem;
    z-index: 1000;
    display: flex;
    align-items: flex-end;
    gap: 0.75rem;
  }

  .chat-icon-button {
    position: relative;
    width: 56px;
    height: 56px;
    background: linear-gradient(135deg, #6366f1, #4f46e5);
    color: white;
    border: none;
    border-radius: 50%;
    box-shadow: 0 4px 12px rgba(99, 102, 241, 0.4);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.2s ease;
  }

  .chat-icon-button:hover {
    transform: scale(1.05);
    box-shadow: 0 6px 16px rgba(99, 102, 241, 0.5);
  }

  .unread-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    background: #ef4444;
    color: white;
    font-size: 0.7rem;
    font-weight: 700;
    border-radius: 999px;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px solid white;
  }
</style>
