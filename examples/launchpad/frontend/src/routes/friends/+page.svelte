<script lang="ts">
  import { user } from '$lib/stores/auth';
  import {
    sendFriendshipRequest,
    respondToFriendRequest,
    unfriend,
    type Message,
    type Friend,
    type FriendSuggestion
  } from '$lib/stores/messaging';
  import { presenceStore } from '$lib/stores/presence';
  import { invalidateAll } from '$app/navigation';
  import { goto } from '$app/navigation';
  import {
    Users,
    UserPlus,
    UserCheck,
    Sparkles,
    Send,
    AlertCircle,
    CheckCircle,
    Clock,
    XCircle,
    MessageCircle,
    UserMinus,
    Check,
    X
  } from 'lucide-svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  // Tab state
  type Tab = 'friends' | 'requests' | 'suggestions';
  let activeTab = $state<Tab>('friends');

  // Form state for sending requests
  let recipientEmail = $state('');
  let message = $state('');
  let sending = $state(false);
  let formError = $state<string | null>(null);
  let successMessage = $state<string | null>(null);

  // Action states
  let processingId = $state<string | null>(null);

  // Computed tab counts
  const friendCount = $derived(data.friends?.length ?? 0);
  const requestCount = $derived(data.requests?.length ?? 0);
  const suggestionCount = $derived(data.suggestions?.length ?? 0);

  async function handleSendRequest(e: Event) {
    e.preventDefault();

    if (!recipientEmail.trim()) {
      formError = 'Please enter an email address';
      return;
    }

    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(recipientEmail.trim())) {
      formError = 'Please enter a valid email address';
      return;
    }

    if (recipientEmail.trim().toLowerCase() === $user?.email?.toLowerCase()) {
      formError = "You can't send a friend request to yourself";
      return;
    }

    sending = true;
    formError = null;
    successMessage = null;

    const result = await sendFriendshipRequest(recipientEmail.trim(), message.trim() || undefined);

    if (result.success) {
      successMessage = 'Friend request sent!';
      recipientEmail = '';
      message = '';
      await invalidateAll();
    } else {
      formError = result.error || 'Failed to send request';
    }

    sending = false;
  }

  async function handleAccept(msg: Message) {
    processingId = msg.id;
    formError = null;
    const result = await respondToFriendRequest(msg, true);
    if (result.success) {
      successMessage = 'Friend request accepted!';
      await invalidateAll();
    } else {
      formError = result.error || 'Failed to accept request';
    }
    processingId = null;
  }

  async function handleDecline(msg: Message) {
    processingId = msg.id;
    formError = null;
    const result = await respondToFriendRequest(msg, false);
    if (result.success) {
      successMessage = 'Friend request declined';
      await invalidateAll();
    } else {
      formError = result.error || 'Failed to decline request';
    }
    processingId = null;
  }

  async function handleUnfriend(friend: Friend) {
    processingId = friend.id;
    formError = null;
    const result = await unfriend(friend.path);
    if (result.success) {
      successMessage = 'Friend removed';
      await invalidateAll();
    } else {
      formError = result.error || 'Failed to unfriend';
    }
    processingId = null;
  }

  async function handleSendRequestToSuggestion(suggestion: FriendSuggestion) {
    const email = suggestion.properties?.email;
    if (!email) return;

    processingId = suggestion.id;
    formError = null;
    const result = await sendFriendshipRequest(email);
    if (result.success) {
      successMessage = 'Friend request sent!';
      await invalidateAll();
    } else {
      formError = result.error || 'Failed to send request';
    }
    processingId = null;
  }

  function formatDate(dateString?: string): string {
    if (!dateString) return '';
    try {
      const date = new Date(dateString);
      const now = new Date();
      const diffMs = now.getTime() - date.getTime();
      const diffDays = Math.floor(diffMs / 86400000);
      if (diffDays < 1) return 'Today';
      if (diffDays < 7) return `${diffDays}d ago`;
      return date.toLocaleDateString();
    } catch {
      return dateString;
    }
  }

  function getDisplayName(props: { email?: string; display_name?: string } | undefined): string {
    if (!props) return 'Unknown';
    return props.display_name || 'Unknown';
  }
</script>

<svelte:head>
  <title>Friends - Launchpad</title>
</svelte:head>

<div class="friends-page">
  <div class="page-header">
    <div class="header-content">
      <Users size={32} />
      <div>
        <h1>Friends</h1>
        <p class="subtitle">Manage your connections and discover new friends</p>
      </div>
    </div>
  </div>

  {#if !$user}
    <div class="not-logged-in">
      <AlertCircle size={24} />
      <p>Please <a href="/auth/login">log in</a> to manage friends.</p>
    </div>
  {:else}
    <!-- Send Request Form -->
    <section class="send-section">
      <div class="section-header">
        <UserPlus size={20} />
        <h2>Add Friend</h2>
      </div>

      <form class="send-form" onsubmit={handleSendRequest}>
        {#if formError}
          <div class="error-message">
            <AlertCircle size={16} />
            {formError}
          </div>
        {/if}

        {#if successMessage}
          <div class="success-message">
            <CheckCircle size={16} />
            {successMessage}
          </div>
        {/if}

        <div class="form-row">
          <input
            type="email"
            bind:value={recipientEmail}
            placeholder="Enter email address"
            disabled={sending}
            class="input-email"
          />
          <button type="submit" class="btn-send" disabled={sending || !recipientEmail.trim()}>
            <Send size={16} />
            {sending ? 'Sending...' : 'Send'}
          </button>
        </div>

        <textarea
          bind:value={message}
          placeholder="Add a message (optional)"
          rows="2"
          disabled={sending}
          class="input-message"
        ></textarea>
      </form>
    </section>

    <!-- Tabs -->
    <div class="tabs">
      <button
        class="tab"
        class:active={activeTab === 'friends'}
        onclick={() => activeTab = 'friends'}
      >
        <UserCheck size={18} />
        My Friends
        {#if friendCount > 0}
          <span class="tab-count">{friendCount}</span>
        {/if}
      </button>

      <button
        class="tab"
        class:active={activeTab === 'requests'}
        onclick={() => activeTab = 'requests'}
      >
        <UserPlus size={18} />
        Requests
        {#if requestCount > 0}
          <span class="tab-count highlight">{requestCount}</span>
        {/if}
      </button>

      <button
        class="tab"
        class:active={activeTab === 'suggestions'}
        onclick={() => activeTab = 'suggestions'}
      >
        <Sparkles size={18} />
        Suggestions
        {#if suggestionCount > 0}
          <span class="tab-count">{suggestionCount}</span>
        {/if}
      </button>
    </div>

    <!-- Tab Content -->
    <div class="tab-content">
      {#if activeTab === 'friends'}
        {#if data.friends.length === 0}
          <div class="empty-state">
            <Users size={48} />
            <h3>No friends yet</h3>
            <p>Send a friend request to get started!</p>
          </div>
        {:else}
          <div class="friends-list">
            {#each data.friends as friend (friend.id)}
              <div class="friend-card">
                <div class="friend-avatar">
                  {getDisplayName(friend.properties).charAt(0).toUpperCase()}
                  <span
                    class="status-indicator"
                    class:online={presenceStore.isOnline(friend.path)}
                    class:offline={!presenceStore.isOnline(friend.path)}
                  ></span>
                </div>
                <div class="friend-info">
                  <span class="friend-name">{getDisplayName(friend.properties)}</span>
                </div>
                <div class="friend-actions">
                  <button
                    class="btn-message"
                    onclick={() => goto('/inbox')}
                    title="Send message"
                  >
                    <MessageCircle size={16} />
                  </button>
                  <button
                    class="btn-unfriend"
                    onclick={() => handleUnfriend(friend)}
                    disabled={processingId === friend.id}
                    title="Unfriend"
                  >
                    <UserMinus size={16} />
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {:else if activeTab === 'requests'}
        {#if formError}
          <div class="error-message" style="margin-bottom: 1rem;">
            <AlertCircle size={16} />
            {formError}
          </div>
        {/if}
        {#if successMessage}
          <div class="success-message" style="margin-bottom: 1rem;">
            <CheckCircle size={16} />
            {successMessage}
          </div>
        {/if}
        {#if data.requests.length === 0}
          <div class="empty-state">
            <UserPlus size={48} />
            <h3>No pending requests</h3>
            <p>When someone sends you a friend request, it will appear here.</p>
          </div>
        {:else}
          <div class="requests-list">
            {#each data.requests as request (request.id)}
              <div class="request-card">
                <div class="request-avatar">
                  {(request.properties.sender_display_name || request.properties.sender_id || '?').charAt(0).toUpperCase()}
                </div>
                <div class="request-info">
                  <span class="request-name">
                    {request.properties.sender_display_name || request.properties.sender_id || 'Unknown'}
                  </span>
                  {#if request.properties.message || request.properties.body?.message}
                    <p class="request-message">"{request.properties.message || request.properties.body?.message}"</p>
                  {/if}
                  <span class="request-time">{formatDate(request.properties.received_at || request.properties.created_at)}</span>
                </div>
                <div class="request-actions">
                  <button
                    class="btn-accept"
                    onclick={() => handleAccept(request)}
                    disabled={processingId === request.id}
                  >
                    <Check size={16} />
                    Accept
                  </button>
                  <button
                    class="btn-decline"
                    onclick={() => handleDecline(request)}
                    disabled={processingId === request.id}
                  >
                    <X size={16} />
                    Decline
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        {#if data.sentRequests && data.sentRequests.length > 0}
          <div class="sent-requests">
            <h3 class="sent-header">
              <Clock size={16} />
              Sent Requests
            </h3>
            <div class="sent-list">
              {#each data.sentRequests as sent (sent.id)}
                <div class="sent-card">
                  <span class="sent-to">Request sent</span>
                  <span class="sent-status">
                    {#if sent.properties.status === 'pending'}
                      <Clock size={14} /> Pending
                    {:else}
                      <CheckCircle size={14} /> Sent
                    {/if}
                  </span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      {:else if activeTab === 'suggestions'}
        {#if data.suggestions.length === 0}
          <div class="empty-state">
            <Sparkles size={48} />
            <h3>No suggestions yet</h3>
            <p>Add some friends to see suggestions based on mutual connections.</p>
          </div>
        {:else}
          <div class="suggestions-list">
            {#each data.suggestions as suggestion (suggestion.id)}
              <div class="suggestion-card">
                <div class="suggestion-avatar">
                  {getDisplayName(suggestion.properties).charAt(0).toUpperCase()}
                </div>
                <div class="suggestion-info">
                  <span class="suggestion-name">{getDisplayName(suggestion.properties)}</span>
                  {#if suggestion.degree}
                    <span class="connection-degree">
                      {suggestion.degree === 2 ? '2nd' : '3rd'}
                    </span>
                  {/if}
                </div>
                {#if suggestion.hasPendingRequest}
                  <button class="btn-pending" disabled>
                    <Clock size={16} />
                    Pending
                  </button>
                {:else}
                  <button
                    class="btn-add-friend"
                    onclick={() => handleSendRequestToSuggestion(suggestion)}
                    disabled={processingId === suggestion.id}
                  >
                    <UserPlus size={16} />
                    Add
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .friends-page {
    max-width: 800px;
    margin: 0 auto;
    padding: 2rem;
  }

  .page-header {
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

  /* Send Section */
  .send-section {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .section-header h2 {
    font-size: 1rem;
    font-weight: 600;
    color: #1f2937;
    margin: 0;
  }

  .section-header :global(svg) {
    color: #6366f1;
  }

  .send-form {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .form-row {
    display: flex;
    gap: 0.5rem;
  }

  .input-email {
    flex: 1;
    padding: 0.625rem 0.875rem;
    border: 1px solid #d1d5db;
    border-radius: 8px;
    font-size: 0.875rem;
  }

  .input-email:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .input-message {
    padding: 0.625rem 0.875rem;
    border: 1px solid #d1d5db;
    border-radius: 8px;
    font-size: 0.875rem;
    resize: none;
  }

  .input-message:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .btn-send {
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
    white-space: nowrap;
  }

  .btn-send:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-send:disabled {
    background: #9ca3af;
    cursor: not-allowed;
  }

  .error-message,
  .success-message {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    font-size: 0.875rem;
  }

  .error-message {
    background: #fef2f2;
    color: #dc2626;
  }

  .success-message {
    background: #f0fdf4;
    color: #16a34a;
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 0.25rem;
    background: #f3f4f6;
    padding: 0.25rem;
    border-radius: 10px;
    margin-bottom: 1.5rem;
  }

  .tab {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    background: transparent;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    color: #6b7280;
    cursor: pointer;
    transition: all 0.2s;
  }

  .tab:hover {
    color: #374151;
  }

  .tab.active {
    background: white;
    color: #1f2937;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .tab-count {
    font-size: 0.75rem;
    padding: 0.125rem 0.5rem;
    background: #e5e7eb;
    border-radius: 999px;
  }

  .tab.active .tab-count {
    background: #6366f1;
    color: white;
  }

  .tab-count.highlight {
    background: #dc2626;
    color: white;
  }

  /* Tab Content */
  .tab-content {
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 12px;
    padding: 1.5rem;
  }

  .empty-state {
    text-align: center;
    padding: 3rem 2rem;
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

  /* Friends List */
  .friends-list,
  .requests-list,
  .suggestions-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .friend-card,
  .request-card,
  .suggestion-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1rem;
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 10px;
  }

  .friend-avatar {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    color: white;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: 1.125rem;
    flex-shrink: 0;
    position: relative;
  }

  .status-indicator {
    position: absolute;
    bottom: 2px;
    right: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid white;
  }

  .status-indicator.online {
    background-color: #10b981;
  }

  .status-indicator.offline {
    background-color: #9ca3af;
  }

  .friend-info,
  .request-info,
  .suggestion-info {
    flex: 1;
    min-width: 0;
  }

  .friend-name,
  .request-name,
  .suggestion-name {
    display: block;
    font-weight: 600;
    color: #1f2937;
  }

  .friend-email,
  .request-email,
  .suggestion-email {
    display: block;
    font-size: 0.875rem;
    color: #6b7280;
  }

  .request-message {
    font-size: 0.875rem;
    color: #4b5563;
    font-style: italic;
    margin: 0.5rem 0;
  }

  .request-time {
    display: block;
    font-size: 0.75rem;
    color: #9ca3af;
    margin-top: 0.25rem;
  }

  .connection-degree {
    display: inline-flex;
    align-items: center;
    font-size: 0.7rem;
    font-weight: 600;
    padding: 0.125rem 0.375rem;
    background: #e0e7ff;
    color: #4338ca;
    border-radius: 4px;
    margin-top: 0.25rem;
  }

  .friend-actions,
  .request-actions {
    display: flex;
    gap: 0.5rem;
  }

  .btn-message,
  .btn-unfriend {
    padding: 0.5rem;
    background: #f3f4f6;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    color: #6b7280;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-message:hover {
    background: #6366f1;
    border-color: #6366f1;
    color: white;
  }

  .btn-unfriend:hover {
    background: #fef2f2;
    border-color: #fecaca;
    color: #dc2626;
  }

  .btn-accept {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 1rem;
    background: #10b981;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
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
    padding: 0.5rem 1rem;
    background: white;
    color: #dc2626;
    border: 1px solid #fecaca;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-decline:hover:not(:disabled) {
    background: #fef2f2;
  }

  .btn-add-friend {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.5rem 1rem;
    background: #6366f1;
    color: white;
    border: none;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-add-friend:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-pending {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.5rem 1rem;
    background: #f3f4f6;
    color: #6b7280;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: not-allowed;
  }

  .btn-accept:disabled,
  .btn-decline:disabled,
  .btn-add-friend:disabled,
  .btn-unfriend:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  /* Sent Requests */
  .sent-requests {
    margin-top: 2rem;
    padding-top: 1.5rem;
    border-top: 1px solid #e5e7eb;
  }

  .sent-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    font-weight: 600;
    color: #6b7280;
    margin: 0 0 1rem;
  }

  .sent-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .sent-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.75rem 1rem;
    background: #f9fafb;
    border-radius: 8px;
    font-size: 0.875rem;
  }

  .sent-to {
    color: #4b5563;
  }

  .sent-status {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    color: #6b7280;
    font-size: 0.75rem;
  }

  @media (max-width: 640px) {
    .friends-page {
      padding: 1rem;
    }

    .tabs {
      flex-direction: column;
    }

    .friend-card,
    .request-card,
    .suggestion-card {
      flex-direction: column;
      text-align: center;
    }

    .friend-actions,
    .request-actions {
      width: 100%;
      justify-content: center;
    }

    .form-row {
      flex-direction: column;
    }

    .btn-send {
      width: 100%;
      justify-content: center;
    }
  }
</style>
