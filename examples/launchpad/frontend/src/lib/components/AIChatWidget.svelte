<script lang="ts">
  import { page } from '$app/stores';
  import { Bot } from 'lucide-svelte';
  import { user } from '$lib/stores/auth';
  import {
    aiChatStore,
    isAIChatOpen,
    isAIChatMinimized,
    activeAIConversation,
    activeAIMessages,
    isWaitingForAIResponse,
  } from '$lib/stores/ai-chat';
  import AIChatPopup from './AIChatPopup.svelte';

  const currentUser = $derived($user);
  const isLoggedIn = $derived(!!currentUser?.home);
  // Hide on inbox page
  const isOnInboxPage = $derived($page.url.pathname.startsWith('/inbox'));

  // Initialize store when user logs in
  $effect(() => {
    if (isLoggedIn) {
      aiChatStore.init();
    } else {
      aiChatStore.reset();
    }
  });

  function handleIconClick() {
    aiChatStore.toggle();
  }
</script>

{#if isLoggedIn && !isOnInboxPage}
  <div class="ai-chat-widget-container">
    <!-- Chat popup -->
    {#if $isAIChatOpen}
      <AIChatPopup isMinimized={$isAIChatMinimized} />
    {/if}

    <!-- AI Chat icon button -->
    <button
      onclick={handleIconClick}
      class="ai-chat-icon-button"
      title="AI Assistant"
    >
      <Bot size={24} />

      <!-- Waiting indicator -->
      {#if $isWaitingForAIResponse}
        <span class="waiting-indicator"></span>
      {/if}
    </button>
  </div>
{/if}

<style>
  .ai-chat-widget-container {
    position: fixed;
    bottom: 1rem;
    right: 5rem; /* Offset from user chat widget */
    z-index: 1000;
    display: flex;
    align-items: flex-end;
    gap: 0.75rem;
  }

  .ai-chat-icon-button {
    position: relative;
    width: 56px;
    height: 56px;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border: none;
    border-radius: 50%;
    box-shadow: 0 4px 12px rgba(139, 92, 246, 0.4);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.2s ease;
  }

  .ai-chat-icon-button:hover {
    transform: scale(1.05);
    box-shadow: 0 6px 16px rgba(139, 92, 246, 0.5);
  }

  .waiting-indicator {
    position: absolute;
    top: -2px;
    right: -2px;
    width: 14px;
    height: 14px;
    background: #22c55e;
    border-radius: 50%;
    border: 2px solid white;
    animation: pulse 1.5s infinite;
  }

  @keyframes pulse {
    0%, 100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.7;
      transform: scale(1.1);
    }
  }
</style>
