<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { query } from '$lib/raisin';
  import { inbox } from '$lib/stores/inbox-store.svelte';
  import type { Message, Friend, FriendSuggestion } from './+page';
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
  import SqlDisplay from '$lib/components/SqlDisplay.svelte';

  const ACCESS_CONTROL = 'raisin:access_control';

  let { data }: { data: PageData } = $props();

  type Tab = 'friends' | 'requests' | 'suggestions';
  let activeTab = $state<Tab>('friends');

  let recipientEmail = $state('');
  let message = $state('');
  let sending = $state(false);
  let formError = $state<string | null>(null);
  let successMessage = $state<string | null>(null);
  let processingId = $state<string | null>(null);

  const friendCount = $derived(data.friends?.length ?? 0);
  const requestCount = $derived(data.requests?.length ?? 0);
  const suggestionCount = $derived(data.suggestions?.length ?? 0);

  // ---------------------------------------------------------------------------
  // Friendship operations (inlined from deleted messaging-utils.ts)
  // ---------------------------------------------------------------------------

  async function sendFriendshipRequest(
    recipientEmail: string,
    msg?: string,
  ): Promise<{ success: boolean; error?: string }> {
    const currentUser = $user;
    if (!currentUser?.home) return { success: false, error: 'Not logged in' };
    const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
    try {
      const messageName = `rel-req-${Date.now()}`;
      const messagePath = `${homePath}/outbox/${messageName}`;
      const properties = {
        message_type: 'relationship_request',
        subject: 'Friendship Request',
        status: 'pending',
        relation_type: 'FRIENDS_WITH',
        message: msg || null,
        body: { message: msg || null },
        sender_id: currentUser.id,
        recipient_email: recipientEmail,
        created_at: new Date().toISOString(),
      };
      await query(`INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties) VALUES ($1, 'raisin:Message', $2::jsonb)`, [messagePath, JSON.stringify(properties)]);
      return { success: true };
    } catch (err) {
      return { success: false, error: (err as any).message || 'Failed' };
    }
  }

  async function respondToFriendRequest(
    originalMessage: { id: string; path: string; properties: { original_request_id?: string; body?: { original_request_id?: string } } },
    accept: boolean,
  ): Promise<{ success: boolean; error?: string }> {
    const currentUser = $user;
    if (!currentUser?.home) return { success: false, error: 'Not logged in' };
    const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
    const originalRequestId = originalMessage.properties.original_request_id || originalMessage.properties.body?.original_request_id;
    if (!originalRequestId) return { success: false, error: 'Missing original request reference' };
    try {
      const messageName = `friend-resp-${Date.now()}`;
      const messagePath = `${homePath}/outbox/${messageName}`;
      const properties = {
        message_type: 'relationship_response',
        subject: accept ? 'Friend Request Accepted' : 'Friend Request Declined',
        status: 'pending',
        accepted: accept,
        original_request_id: originalRequestId,
        body: { response: accept ? 'accepted' : 'declined', original_request_id: originalRequestId },
        sender_id: currentUser.id,
        created_at: new Date().toISOString(),
      };
      await query(`INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties) VALUES ($1, 'raisin:Message', $2::jsonb)`, [messagePath, JSON.stringify(properties)]);
      // Mark the original message as read
      await query(`UPDATE '${ACCESS_CONTROL}' SET properties = properties || CAST('{"status": "read"}' AS JSONB) WHERE path LIKE $1`, [originalMessage.path]);
      return { success: true };
    } catch (err) {
      return { success: false, error: (err as any).message };
    }
  }

  async function unfriendUser(friendPath: string): Promise<{ success: boolean; error?: string }> {
    const currentUser = $user;
    if (!currentUser?.home) return { success: false, error: 'Not logged in' };
    const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
    try {
      await query(`UNRELATE FROM path='${homePath}' IN WORKSPACE '${ACCESS_CONTROL}' TO path='${friendPath}' IN WORKSPACE '${ACCESS_CONTROL}' TYPE 'FRIENDS_WITH'`);
      await query(`UNRELATE FROM path='${friendPath}' IN WORKSPACE '${ACCESS_CONTROL}' TO path='${homePath}' IN WORKSPACE '${ACCESS_CONTROL}' TYPE 'FRIENDS_WITH'`);
      return { success: true };
    } catch (err) {
      return { success: false, error: (err as any).message };
    }
  }

  // ---------------------------------------------------------------------------
  // Event handlers
  // ---------------------------------------------------------------------------

  async function handleSendRequest(e: Event) {
    e.preventDefault();
    if (!recipientEmail.trim()) { formError = 'Please enter an email address'; return; }
    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(recipientEmail.trim())) { formError = 'Please enter a valid email address'; return; }
    if (recipientEmail.trim().toLowerCase() === $user?.email?.toLowerCase()) { formError = "You can't send a friend request to yourself"; return; }

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

  async function handleAccept(msg: any) {
    processingId = msg.id;
    formError = null;
    const result = await respondToFriendRequest(msg, true);
    if (result.success) { successMessage = 'Friend request accepted!'; await invalidateAll(); }
    else { formError = result.error || 'Failed to accept request'; }
    processingId = null;
  }

  async function handleDecline(msg: any) {
    processingId = msg.id;
    formError = null;
    const result = await respondToFriendRequest(msg, false);
    if (result.success) { successMessage = 'Friend request declined'; await invalidateAll(); }
    else { formError = result.error || 'Failed to decline request'; }
    processingId = null;
  }

  async function handleUnfriend(friend: any) {
    processingId = friend.id;
    formError = null;
    const result = await unfriendUser(friend.path);
    if (result.success) { successMessage = 'Friend removed'; await invalidateAll(); }
    else { formError = result.error || 'Failed to unfriend'; }
    processingId = null;
  }

  async function handleSendRequestToSuggestion(suggestion: any) {
    const email = suggestion.properties?.email;
    if (!email) return;
    processingId = suggestion.id;
    formError = null;
    const result = await sendFriendshipRequest(email);
    if (result.success) { successMessage = 'Friend request sent!'; await invalidateAll(); }
    else { formError = result.error || 'Failed to send request'; }
    processingId = null;
  }

  async function handleMessageFriend(friend: any) {
    processingId = friend.id;
    formError = null;
    try {
      const convo = await inbox.createConversation(friend.id);
      if (convo) {
        const name = convo.conversationPath.split('/').pop() || convo.conversationPath;
        goto(`/inbox/${name}`);
      }
    } catch (err) {
      formError = 'Failed to create conversation';
    }
    processingId = null;
  }

  function formatDate(dateString?: string): string {
    if (!dateString) return '';
    try {
      const date = new Date(dateString);
      const now = new Date();
      const diffDays = Math.floor((now.getTime() - date.getTime()) / 86400000);
      if (diffDays < 1) return 'Today';
      if (diffDays < 7) return `${diffDays}d ago`;
      return date.toLocaleDateString();
    } catch { return dateString; }
  }

  function getDisplayName(props: { email?: string; display_name?: string } | undefined): string {
    if (!props) return 'Unknown';
    return props.display_name || 'Unknown';
  }
</script>

<svelte:head>
  <title>Friends - Nachtkultur</title>
</svelte:head>

<div class="friends-page">
  <div class="page-header">
    <div class="header-content">
      <div class="header-icon">
        <Users size={20} />
      </div>
      <div>
        <h1>Friends</h1>
        <p class="subtitle">Manage your connections</p>
      </div>
    </div>
  </div>

  {#if !$user}
    <div class="not-logged-in">
      <AlertCircle size={18} />
      <p>Please <a href="/auth/login">sign in</a> to manage friends.</p>
    </div>
  {:else}
    <!-- Send Request Form -->
    <section class="send-section">
      <div class="section-header">
        <UserPlus size={16} />
        <h2>Add Friend</h2>
      </div>

      <form class="send-form" onsubmit={handleSendRequest}>
        {#if formError}
          <div class="error-message"><AlertCircle size={14} /> {formError}</div>
        {/if}
        {#if successMessage}
          <div class="success-message"><CheckCircle size={14} /> {successMessage}</div>
        {/if}

        <div class="form-row">
          <input type="email" bind:value={recipientEmail} placeholder="Enter email address" disabled={sending} class="input-email" />
          <button type="submit" class="btn-send" disabled={sending || !recipientEmail.trim()}>
            <Send size={14} />
            {sending ? 'Sending...' : 'Send'}
          </button>
        </div>

        <textarea bind:value={message} placeholder="Add a message (optional)" rows="2" disabled={sending} class="input-message"></textarea>
      </form>
    </section>

    <!-- Tabs -->
    <div class="tabs">
      <button class="tab" class:active={activeTab === 'friends'} onclick={() => activeTab = 'friends'}>
        <UserCheck size={16} /> My Friends
        {#if friendCount > 0}<span class="tab-count">{friendCount}</span>{/if}
      </button>
      <button class="tab" class:active={activeTab === 'requests'} onclick={() => activeTab = 'requests'}>
        <UserPlus size={16} /> Requests
        {#if requestCount > 0}<span class="tab-count highlight">{requestCount}</span>{/if}
      </button>
      <button class="tab" class:active={activeTab === 'suggestions'} onclick={() => activeTab = 'suggestions'}>
        <Sparkles size={16} /> Suggestions
        {#if suggestionCount > 0}<span class="tab-count">{suggestionCount}</span>{/if}
      </button>
    </div>

    <!-- Tab Content -->
    <div class="tab-content">
      {#if activeTab === 'friends'}
        {#if data.queries?.friends}
          <SqlDisplay sql={data.queries.friends} title="Get Friends (GRAPH_TABLE)" />
        {/if}
        {#if data.friends.length === 0}
          <div class="empty-state">
            <Users size={40} />
            <h3>No friends yet</h3>
            <p>Send a friend request to get started!</p>
          </div>
        {:else}
          <div class="friends-list">
            {#each data.friends as friend (friend.id)}
              <div class="friend-card">
                <div class="friend-avatar">
                  {getDisplayName(friend.properties).charAt(0).toUpperCase()}
                </div>
                <div class="friend-info">
                  <span class="friend-name">{getDisplayName(friend.properties)}</span>
                </div>
                <div class="friend-actions">
                  <button class="btn-icon" onclick={() => handleMessageFriend(friend)} disabled={processingId === friend.id} title="Send message">
                    <MessageCircle size={15} />
                  </button>
                  <button class="btn-icon danger" onclick={() => handleUnfriend(friend)} disabled={processingId === friend.id} title="Unfriend">
                    <UserMinus size={15} />
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {:else if activeTab === 'requests'}
        {#if data.queries?.requests}
          <SqlDisplay sql={data.queries.requests} title="Pending Requests (DESCENDANT_OF)" />
        {/if}
        {#if formError}
          <div class="error-message" style="margin-bottom: 1rem;"><AlertCircle size={14} /> {formError}</div>
        {/if}
        {#if successMessage}
          <div class="success-message" style="margin-bottom: 1rem;"><CheckCircle size={14} /> {successMessage}</div>
        {/if}
        {#if data.requests.length === 0}
          <div class="empty-state">
            <UserPlus size={40} />
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
                  <button class="btn-accept" onclick={() => handleAccept(request)} disabled={processingId === request.id}>
                    <Check size={14} /> Accept
                  </button>
                  <button class="btn-decline" onclick={() => handleDecline(request)} disabled={processingId === request.id}>
                    <X size={14} /> Decline
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        {#if data.sentRequests && data.sentRequests.length > 0}
          <div class="sent-requests">
            <h3 class="sent-header"><Clock size={14} /> Sent Requests</h3>
            {#if data.queries?.sentRequests}
              <SqlDisplay sql={data.queries.sentRequests} title="Sent Requests (DESCENDANT_OF with OR)" />
            {/if}
            <div class="sent-list">
              {#each data.sentRequests as sent (sent.id)}
                <div class="sent-card">
                  <span class="sent-to">Request sent</span>
                  <span class="sent-status">
                    {#if sent.properties.status === 'pending'}
                      <Clock size={12} /> Pending
                    {:else}
                      <CheckCircle size={12} /> Sent
                    {/if}
                  </span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      {:else if activeTab === 'suggestions'}
        {#if data.queries?.suggestions}
          <SqlDisplay sql={data.queries.suggestions} title="Friend Suggestions (Variable-Length Paths)" />
        {/if}
        {#if data.suggestions.length === 0}
          <div class="empty-state">
            <Sparkles size={40} />
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
                    <Clock size={14} /> Pending
                  </button>
                {:else}
                  <button class="btn-add-friend" onclick={() => handleSendRequestToSuggestion(suggestion)} disabled={processingId === suggestion.id}>
                    <UserPlus size={14} /> Add
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
    padding: 2.5rem 2rem;
    animation: fadeInUp 0.4s ease both;
  }

  .page-header { margin-bottom: 2rem; }

  .header-content {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
  }

  .header-icon {
    width: 40px;
    height: 40px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-accent);
    flex-shrink: 0;
  }

  .page-header h1 {
    font-family: var(--font-display);
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0;
    letter-spacing: -0.02em;
  }

  .subtitle {
    color: var(--color-text-muted);
    margin: 0.25rem 0 0;
    font-size: 0.85rem;
  }

  .not-logged-in {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    background: var(--color-warning-muted);
    color: var(--color-warning);
    padding: 1rem 1.25rem;
    border-radius: var(--radius-md);
    border: 1px solid rgba(245, 158, 11, 0.15);
    font-size: 0.9rem;
  }

  .not-logged-in a { color: var(--color-accent); }

  /* Send Section */
  .send-section {
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
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
    font-family: var(--font-display);
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0;
  }

  .section-header :global(svg) {
    color: var(--color-accent);
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
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    font-size: 0.85rem;
    color: var(--color-text);
    font-family: var(--font-body);
  }

  .input-email:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .input-email::placeholder { color: var(--color-text-muted); }

  .input-message {
    padding: 0.625rem 0.875rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    font-size: 0.85rem;
    color: var(--color-text);
    resize: none;
    font-family: var(--font-body);
  }

  .input-message:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .input-message::placeholder { color: var(--color-text-muted); }

  .btn-send {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-md);
    font-weight: 600;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.2s;
    white-space: nowrap;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    font-family: var(--font-body);
  }

  .btn-send:hover:not(:disabled) { background: var(--color-accent-hover); }
  .btn-send:disabled { opacity: 0.4; cursor: not-allowed; }

  .error-message, .success-message {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius-md);
    font-size: 0.85rem;
  }

  .error-message {
    background: var(--color-error-muted);
    color: var(--color-error);
    border: 1px solid rgba(239, 68, 68, 0.15);
  }

  .success-message {
    background: var(--color-success-muted);
    color: var(--color-success);
    border: 1px solid rgba(62, 207, 142, 0.15);
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 2px;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    padding: 4px;
    border-radius: var(--radius-md);
    margin-bottom: 1.5rem;
  }

  .tab {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: 0.625rem 0.75rem;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    font-weight: 500;
    color: var(--color-text-muted);
    cursor: pointer;
    transition: all 0.2s;
    font-family: var(--font-body);
  }

  .tab:hover { color: var(--color-text-secondary); }

  .tab.active {
    background: var(--color-surface);
    color: var(--color-text-heading);
  }

  .tab-count {
    font-size: 0.65rem;
    padding: 0.1rem 0.375rem;
    background: var(--color-surface);
    border-radius: 999px;
    color: var(--color-text-secondary);
  }

  .tab.active .tab-count {
    background: var(--color-accent);
    color: var(--color-bg);
  }

  .tab-count.highlight {
    background: var(--color-rose);
    color: white;
  }

  /* Tab Content */
  .tab-content {
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: 1.5rem;
  }

  .empty-state {
    text-align: center;
    padding: 3rem 2rem;
  }

  .empty-state :global(svg) {
    color: var(--color-text-muted);
    margin-bottom: 1rem;
  }

  .empty-state h3 {
    font-family: var(--font-display);
    font-size: 1rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0 0 0.375rem;
  }

  .empty-state p {
    color: var(--color-text-muted);
    margin: 0;
    font-size: 0.85rem;
  }

  /* Lists */
  .friends-list, .requests-list, .suggestions-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .friend-card, .request-card, .suggestion-card {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    padding: 0.875rem 1rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    transition: border-color 0.2s;
  }

  .friend-card:hover, .request-card:hover, .suggestion-card:hover {
    border-color: var(--color-border);
  }

  .friend-avatar, .request-avatar, .suggestion-avatar {
    width: 42px;
    height: 42px;
    border-radius: 50%;
    background: linear-gradient(135deg, var(--color-accent), var(--color-rose));
    color: var(--color-bg);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 0.95rem;
    font-family: var(--font-display);
    flex-shrink: 0;
  }

  .friend-info, .request-info, .suggestion-info {
    flex: 1;
    min-width: 0;
  }

  .friend-name, .request-name, .suggestion-name {
    display: block;
    font-weight: 600;
    color: var(--color-text-heading);
    font-size: 0.875rem;
  }

  .request-message {
    font-size: 0.825rem;
    color: var(--color-text-secondary);
    font-style: italic;
    margin: 0.375rem 0;
  }

  .request-time {
    display: block;
    font-size: 0.7rem;
    color: var(--color-text-muted);
    margin-top: 0.2rem;
  }

  .connection-degree {
    display: inline-flex;
    font-size: 0.65rem;
    font-weight: 600;
    padding: 0.1rem 0.375rem;
    background: var(--color-accent-muted);
    color: var(--color-accent);
    border-radius: 3px;
    margin-top: 0.25rem;
    letter-spacing: 0.02em;
  }

  .friend-actions, .request-actions {
    display: flex;
    gap: 0.375rem;
  }

  .btn-icon {
    padding: 0.45rem;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-icon:hover {
    background: var(--color-accent-muted);
    border-color: var(--color-border-accent);
    color: var(--color-accent);
  }

  .btn-icon.danger:hover {
    background: var(--color-error-muted);
    border-color: rgba(239, 68, 68, 0.3);
    color: var(--color-error);
  }

  .btn-accept {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.45rem 0.875rem;
    background: var(--color-success);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-accept:hover:not(:disabled) { filter: brightness(1.1); }

  .btn-decline {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.45rem 0.875rem;
    background: transparent;
    color: var(--color-error);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-decline:hover:not(:disabled) { background: var(--color-error-muted); }

  .btn-add-friend {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.45rem 0.875rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-add-friend:hover:not(:disabled) { background: var(--color-accent-hover); }

  .btn-pending {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.45rem 0.875rem;
    background: var(--color-surface);
    color: var(--color-text-muted);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.8rem;
    font-weight: 500;
    cursor: not-allowed;
  }

  .btn-accept:disabled, .btn-decline:disabled, .btn-add-friend:disabled, .btn-icon:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* Sent Requests */
  .sent-requests {
    margin-top: 1.5rem;
    padding-top: 1.5rem;
    border-top: 1px solid var(--color-border-subtle);
  }

  .sent-header {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-family: var(--font-display);
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--color-text-secondary);
    margin: 0 0 1rem;
  }

  .sent-list {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .sent-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.625rem 0.875rem;
    background: var(--color-surface);
    border-radius: var(--radius-sm);
    font-size: 0.825rem;
  }

  .sent-to { color: var(--color-text-secondary); }

  .sent-status {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--color-text-muted);
    font-size: 0.75rem;
  }

  @media (max-width: 640px) {
    .friends-page { padding: 1.5rem 1rem; }
    .tabs { flex-direction: column; }

    .friend-card, .request-card, .suggestion-card {
      flex-direction: column;
      text-align: center;
    }

    .friend-actions, .request-actions {
      width: 100%;
      justify-content: center;
    }

    .form-row { flex-direction: column; }
    .btn-send { width: 100%; justify-content: center; }
  }
</style>
