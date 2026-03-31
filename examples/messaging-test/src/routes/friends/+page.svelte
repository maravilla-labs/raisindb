<script lang="ts">
  import { onMount } from 'svelte';
  import { user } from '$lib/stores/auth';
  import { messagesStore } from '$lib/stores/messages';
  import { query, ACCESS_CONTROL_WORKSPACE, type UserNode, type MessageNode } from '$lib/raisin';
  import {
    Users, UserPlus, UserMinus, RefreshCw, AlertCircle, Search,
    CheckCircle, XCircle, Clock, Send, Heart
  } from 'lucide-svelte';

  type Tab = 'friends' | 'pending' | 'find';
  let activeTab = $state<Tab>('friends');
  let loading = $state(true);
  let error = $state<string | null>(null);
  let actionLoading = $state<string | null>(null);
  let actionResult = $state<{ success: boolean; message: string } | null>(null);

  // Data
  let friends = $state<Array<{ id: string; path: string; display_name?: string; relation_type: string }>>([]);
  let incomingRequests = $state<MessageNode[]>([]);
  let outgoingRequests = $state<MessageNode[]>([]);
  let allUsers = $state<UserNode[]>([]);
  let searchQuery = $state('');

  function getUserHomePath(): string {
    if (!$user?.home) return '';
    return $user.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');
  }

  async function loadFriends() {
    if (!$user) return;

    try {
      // Load friends using graph query - bidirectional FRIENDS_WITH edges
      const result = await query<{ id: string; path: string; properties: Record<string, unknown>; relation_type: string }>(`
        SELECT * FROM GRAPH_TABLE(
          MATCH (me)-[r:FRIENDS_WITH|PARENT_OF|GUARDIAN_OF|MANAGER_OF]->(friend)
          WHERE me.id = '${$user.id}'
          COLUMNS (friend.id AS id, friend.path AS path, friend.properties AS properties, type(r) AS relation_type)
        ) AS g
      `);

      friends = result.map(r => ({
        id: r.id,
        path: r.path,
        display_name: (r.properties as Record<string, unknown>)?.display_name as string,
        relation_type: r.relation_type
      }));
    } catch (err) {
      console.error('[friends] Failed to load friends:', err);
      // Graph queries might fail if no edges exist, that's OK
      friends = [];
    }
  }

  async function loadPendingRequests() {
    if (!$user?.home) return;

    const homePath = getUserHomePath();

    try {
      // Incoming relationship requests (in my inbox)
      const incoming = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('${homePath}/inbox')
          AND node_type = 'raisin:Message'
          AND properties->>'message_type' = 'relationship_request_received'
          AND properties->>'status' IN ('pending', 'delivered')
        ORDER BY properties->>'created_at' DESC
      `);
      incomingRequests = incoming;

      // Outgoing relationship requests (in my sent)
      const outgoing = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('${homePath}/sent')
          AND node_type = 'raisin:Message'
          AND properties->>'message_type' = 'relationship_request'
          AND properties->>'status' IN ('pending', 'sent', 'delivered')
        ORDER BY properties->>'created_at' DESC
      `);
      outgoingRequests = outgoing;
    } catch (err) {
      console.error('[friends] Failed to load pending requests:', err);
    }
  }

  async function loadAllUsers() {
    try {
      const result = await query<UserNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('/users')
          AND node_type = 'raisin:User'
        ORDER BY properties->>'display_name' ASC, path ASC
      `);
      allUsers = result;
    } catch (err) {
      console.error('[friends] Failed to load users:', err);
    }
  }

  async function refresh() {
    loading = true;
    error = null;
    try {
      await Promise.all([loadFriends(), loadPendingRequests(), loadAllUsers()]);
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to load data';
    } finally {
      loading = false;
    }
  }

  async function sendFriendRequest(recipientId: string) {
    if (!$user?.home) return;

    actionLoading = recipientId;
    actionResult = null;

    try {
      const homePath = getUserHomePath();
      const messageId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const outboxPath = `${homePath}/outbox/${messageId}`;

      const properties = {
        message_type: 'relationship_request',
        status: 'pending',
        created_at: new Date().toISOString(),
        sender_id: $user.id,
        recipient_id: recipientId,
        relation_type: 'FRIENDS_WITH',
        message: 'I would like to be friends with you!'
      };

      await query(`
        INSERT INTO '${ACCESS_CONTROL_WORKSPACE}' (path, node_type, properties)
        VALUES ($1, 'raisin:Message', $2::jsonb)
      `, [outboxPath, JSON.stringify(properties)]);

      actionResult = { success: true, message: 'Friend request sent!' };
      await refresh();
      await messagesStore.refresh();
    } catch (err) {
      console.error('[friends] Failed to send friend request:', err);
      actionResult = { success: false, message: err instanceof Error ? err.message : 'Failed to send request' };
    } finally {
      actionLoading = null;
    }
  }

  async function respondToRequest(request: MessageNode, accepted: boolean) {
    if (!$user?.home) return;

    actionLoading = request.id;
    actionResult = null;

    try {
      const homePath = getUserHomePath();
      const messageId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const outboxPath = `${homePath}/outbox/${messageId}`;

      const properties = {
        message_type: 'relationship_response',
        status: 'pending',
        created_at: new Date().toISOString(),
        sender_id: $user.id,
        accepted,
        original_request_id: request.properties.original_request_id || request.id,
        rejection_reason: accepted ? null : 'Request declined'
      };

      await query(`
        INSERT INTO '${ACCESS_CONTROL_WORKSPACE}' (path, node_type, properties)
        VALUES ($1, 'raisin:Message', $2::jsonb)
      `, [outboxPath, JSON.stringify(properties)]);

      actionResult = { success: true, message: accepted ? 'Request accepted!' : 'Request declined.' };
      await refresh();
      await messagesStore.refresh();
    } catch (err) {
      console.error('[friends] Failed to respond to request:', err);
      actionResult = { success: false, message: err instanceof Error ? err.message : 'Failed to respond' };
    } finally {
      actionLoading = null;
    }
  }

  // Filter users for search
  let filteredUsers = $derived(
    allUsers.filter(u => {
      // Exclude self
      if (u.id === $user?.id) return false;
      // Filter by search query
      if (!searchQuery) return true;
      const q = searchQuery.toLowerCase();
      const displayName = (u.properties.display_name || '').toLowerCase();
      const path = u.path.toLowerCase();
      return displayName.includes(q) || path.includes(q);
    })
  );

  // Check if user is already a friend
  function isFriend(userId: string): boolean {
    return friends.some(f => f.id === userId);
  }

  // Check if there's a pending request
  function hasPendingRequest(userId: string): boolean {
    return outgoingRequests.some(r => r.properties.recipient_id === userId);
  }

  function formatDate(dateStr: string | undefined): string {
    if (!dateStr) return 'Unknown';
    try {
      return new Date(dateStr).toLocaleDateString();
    } catch {
      return dateStr;
    }
  }

  onMount(() => {
    if ($user) {
      refresh();
    }
  });

  $effect(() => {
    if ($user) {
      refresh();
    }
  });
</script>

<div class="friends-page">
  <div class="header">
    <div class="title-section">
      <h1>
        <Users size={28} />
        Friends
      </h1>
      <p>Manage your relationships and friend requests</p>
    </div>
    <button class="btn btn-secondary" onclick={refresh} disabled={loading}>
      <span class:spinning={loading}><RefreshCw size={18} /></span>
      Refresh
    </button>
  </div>

  {#if !$user}
    <div class="alert alert-info">
      <AlertCircle size={18} />
      <span>Login to manage your friends.</span>
    </div>
  {:else}
    {#if actionResult}
      <div class="alert" class:alert-success={actionResult.success} class:alert-error={!actionResult.success}>
        {#if actionResult.success}
          <CheckCircle size={18} />
        {:else}
          <AlertCircle size={18} />
        {/if}
        <span>{actionResult.message}</span>
      </div>
    {/if}

    <div class="tabs-container">
      <div class="tabs">
        <button
          class="tab"
          class:active={activeTab === 'friends'}
          onclick={() => activeTab = 'friends'}
        >
          <Heart size={18} />
          My Friends ({friends.length})
        </button>
        <button
          class="tab"
          class:active={activeTab === 'pending'}
          onclick={() => activeTab = 'pending'}
        >
          <Clock size={18} />
          Pending ({incomingRequests.length + outgoingRequests.length})
        </button>
        <button
          class="tab"
          class:active={activeTab === 'find'}
          onclick={() => activeTab = 'find'}
        >
          <Search size={18} />
          Find Users
        </button>
      </div>
    </div>

    {#if loading}
      <div class="loading">
        <span class="spinning"><RefreshCw size={24} /></span>
        <span>Loading...</span>
      </div>
    {:else}
      {#if activeTab === 'friends'}
        {#if friends.length === 0}
          <div class="empty-state">
            <Users size={48} />
            <h2>No Friends Yet</h2>
            <p>Find users and send friend requests to get started.</p>
            <button class="btn btn-primary" onclick={() => activeTab = 'find'}>
              <Search size={18} />
              Find Users
            </button>
          </div>
        {:else}
          <div class="content-card">
            <div class="friends-list">
              {#each friends as friend (friend.id)}
                <div class="friend-card">
                  <div class="friend-avatar">
                    <Users size={24} />
                  </div>
                  <div class="friend-info">
                    <div class="friend-name">{friend.display_name || friend.path.split('/').pop()}</div>
                    <div class="friend-meta">
                      <span class="badge badge-info">{friend.relation_type}</span>
                      <code>{friend.id}</code>
                    </div>
                  </div>
                  <button
                    class="btn btn-sm btn-danger"
                    title="Remove friend"
                    disabled
                  >
                    <UserMinus size={16} />
                  </button>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      {/if}

      {#if activeTab === 'pending'}
        <div class="content-card">
          {#if incomingRequests.length === 0 && outgoingRequests.length === 0}
            <div class="empty-state-inline">
              <Clock size={32} />
              <p>No pending requests</p>
            </div>
          {:else}
            {#if incomingRequests.length > 0}
              <div class="request-section">
                <h3>Incoming Requests ({incomingRequests.length})</h3>
                <div class="request-list">
                  {#each incomingRequests as request (request.id)}
                    <div class="request-card">
                      <div class="request-info">
                        <div class="request-from">
                          <UserPlus size={18} />
                          <span>From: <strong>{request.properties.sender_display_name || request.properties.sender_id}</strong></span>
                        </div>
                        {#if request.properties.relation_type}
                          <span class="badge badge-info">{request.properties.relation_type}</span>
                        {/if}
                        {#if request.properties.message}
                          <div class="request-message">{request.properties.message}</div>
                        {/if}
                        <div class="request-meta">
                          <span>{formatDate(request.properties.created_at)}</span>
                        </div>
                      </div>
                      <div class="request-actions">
                        <button
                          class="btn btn-sm btn-success"
                          onclick={() => respondToRequest(request, true)}
                          disabled={actionLoading === request.id}
                        >
                          <CheckCircle size={16} />
                          Accept
                        </button>
                        <button
                          class="btn btn-sm btn-danger"
                          onclick={() => respondToRequest(request, false)}
                          disabled={actionLoading === request.id}
                        >
                          <XCircle size={16} />
                          Decline
                        </button>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            {#if outgoingRequests.length > 0}
              <div class="request-section">
                <h3>Outgoing Requests ({outgoingRequests.length})</h3>
                <div class="request-list">
                  {#each outgoingRequests as request (request.id)}
                    <div class="request-card">
                      <div class="request-info">
                        <div class="request-to">
                          <Send size={18} />
                          <span>To: <strong>{request.properties.recipient_id}</strong></span>
                        </div>
                        {#if request.properties.relation_type}
                          <span class="badge badge-info">{request.properties.relation_type}</span>
                        {/if}
                        <span class="badge badge-warning">{request.properties.status}</span>
                        <div class="request-meta">
                          <span>{formatDate(request.properties.created_at)}</span>
                        </div>
                      </div>
                      <button
                        class="btn btn-sm btn-secondary"
                        disabled
                        title="Cancel not implemented"
                      >
                        <XCircle size={16} />
                        Cancel
                      </button>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
          {/if}
        </div>
      {/if}

      {#if activeTab === 'find'}
        <div class="content-card">
          <div class="search-box">
            <Search size={18} />
            <input
              type="text"
              placeholder="Search users by name or path..."
              bind:value={searchQuery}
              class="search-input"
            />
          </div>

          {#if filteredUsers.length === 0}
            <div class="empty-state-inline">
              <Users size={32} />
              <p>No users found</p>
            </div>
          {:else}
            <div class="users-list">
              {#each filteredUsers as u (u.id)}
                <div class="user-card">
                  <div class="user-avatar">
                    <Users size={20} />
                  </div>
                  <div class="user-info">
                    <div class="user-name">{u.properties.display_name || u.name}</div>
                    <code class="user-id">{u.id}</code>
                  </div>
                  <div class="user-actions">
                    {#if isFriend(u.id)}
                      <span class="badge badge-success">Friends</span>
                    {:else if hasPendingRequest(u.id)}
                      <span class="badge badge-warning">Pending</span>
                    {:else}
                      <button
                        class="btn btn-sm btn-primary"
                        onclick={() => sendFriendRequest(u.id)}
                        disabled={actionLoading === u.id}
                      >
                        {#if actionLoading === u.id}
                          <span class="spinning"><RefreshCw size={14} /></span>
                        {:else}
                          <UserPlus size={14} />
                        {/if}
                        Add Friend
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .friends-page {
    max-width: 900px;
    margin: 0 auto;
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 2rem;
  }

  .title-section h1 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1.75rem;
    font-weight: 700;
    color: #111827;
    margin: 0 0 0.5rem 0;
  }

  .title-section p {
    color: #6b7280;
    margin: 0;
  }

  .tabs-container {
    background: white;
    border-radius: 0.75rem 0.75rem 0 0;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    margin-bottom: -1px;
  }

  .tabs {
    display: flex;
    border-bottom: 1px solid #e5e7eb;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 1rem 1.5rem;
    border: none;
    background: transparent;
    color: #6b7280;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }

  .tab:hover {
    color: #374151;
  }

  .tab.active {
    color: #6366f1;
    border-bottom-color: #6366f1;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
    padding: 3rem;
    color: #6b7280;
    background: white;
    border-radius: 0 0 0.75rem 0.75rem;
  }

  .spinning {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .content-card {
    background: white;
    border-radius: 0 0 0.75rem 0.75rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    padding: 1.5rem;
  }

  .empty-state {
    text-align: center;
    padding: 3rem;
    background: white;
    border-radius: 0 0 0.75rem 0.75rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .empty-state :global(svg) {
    color: #9ca3af;
    margin-bottom: 1rem;
  }

  .empty-state h2 {
    margin: 0 0 0.5rem 0;
    color: #374151;
  }

  .empty-state p {
    margin: 0 0 1.5rem 0;
    color: #6b7280;
  }

  .empty-state-inline {
    text-align: center;
    padding: 2rem;
    color: #9ca3af;
  }

  .empty-state-inline p {
    margin: 0.5rem 0 0 0;
  }

  .friends-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .friend-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1rem;
    background: #f9fafb;
    border-radius: 0.5rem;
    border: 1px solid #e5e7eb;
  }

  .friend-avatar {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: #eef2ff;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #6366f1;
  }

  .friend-info {
    flex: 1;
  }

  .friend-name {
    font-weight: 600;
    color: #111827;
    margin-bottom: 0.25rem;
  }

  .friend-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
  }

  .friend-meta code {
    color: #6b7280;
    font-family: monospace;
  }

  .request-section {
    margin-bottom: 1.5rem;
  }

  .request-section:last-child {
    margin-bottom: 0;
  }

  .request-section h3 {
    font-size: 1rem;
    font-weight: 600;
    color: #374151;
    margin: 0 0 1rem 0;
  }

  .request-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .request-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 1rem;
    background: #f9fafb;
    border-radius: 0.5rem;
    border: 1px solid #e5e7eb;
  }

  .request-info {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .request-from,
  .request-to {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: #374151;
  }

  .request-message {
    font-size: 0.875rem;
    color: #6b7280;
    font-style: italic;
  }

  .request-meta {
    font-size: 0.75rem;
    color: #9ca3af;
  }

  .request-actions {
    display: flex;
    gap: 0.5rem;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 0.5rem;
    margin-bottom: 1rem;
  }

  .search-box :global(svg) {
    color: #9ca3af;
  }

  .search-input {
    flex: 1;
    border: none;
    background: transparent;
    font-size: 0.9375rem;
    outline: none;
  }

  .users-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .user-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.75rem 1rem;
    background: #f9fafb;
    border-radius: 0.5rem;
    border: 1px solid #e5e7eb;
  }

  .user-avatar {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: #e5e7eb;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #6b7280;
  }

  .user-info {
    flex: 1;
    min-width: 0;
  }

  .user-name {
    font-weight: 500;
    color: #111827;
  }

  .user-id {
    font-size: 0.75rem;
    color: #6b7280;
    font-family: monospace;
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .user-actions {
    flex-shrink: 0;
  }

  .alert {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1rem;
    border-radius: 0.5rem;
    margin-bottom: 1rem;
  }

  .alert-info {
    background: #eff6ff;
    color: #1e40af;
    border: 1px solid #bfdbfe;
  }

  .alert-success {
    background: #f0fdf4;
    color: #166534;
    border: 1px solid #bbf7d0;
  }

  .alert-error {
    background: #fef2f2;
    color: #991b1b;
    border: 1px solid #fecaca;
  }
</style>
