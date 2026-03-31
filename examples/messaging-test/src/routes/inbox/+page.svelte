<script lang="ts">
  import { onMount } from 'svelte';
  import { user } from '$lib/stores/auth';
  import { inbox, outbox, sent, messagesStore, messagesLoading } from '$lib/stores/messages';
  import { query, ACCESS_CONTROL_WORKSPACE, type MessageNode } from '$lib/raisin';
  import {
    Inbox as InboxIcon, Send, Clock, RefreshCw, AlertCircle,
    CheckCircle, XCircle, Bell, Users, ClipboardList, UserPlus
  } from 'lucide-svelte';

  type Tab = 'inbox' | 'outbox' | 'sent';
  let activeTab = $state<Tab>('inbox');

  // Load notifications separately
  let notifications = $state<MessageNode[]>([]);
  let tasks = $state<Array<{ id: string; path: string; name: string; node_type: string; properties: Record<string, unknown> }>>([]);

  async function loadExtras() {
    if (!$user?.home) return;

    const homePath = $user.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');

    try {
      // Load notifications
      const notifs = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('${homePath}/inbox/notifications')
          AND node_type = 'raisin:Notification'
        ORDER BY properties->>'created_at' DESC
        LIMIT 50
      `);
      notifications = notifs;

      // Load tasks
      const taskList = await query<{ id: string; path: string; name: string; node_type: string; properties: Record<string, unknown> }>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE CHILD_OF('${homePath}/inbox')
          AND node_type = 'raisin:InboxTask'
        ORDER BY properties->>'created_at' DESC
        LIMIT 50
      `);
      tasks = taskList;
    } catch (err) {
      console.error('[inbox] Failed to load extras:', err);
    }
  }

  function getMessageIcon(type: string) {
    switch (type) {
      case 'system_notification': return Bell;
      case 'task_assignment': return ClipboardList;
      case 'relationship_request': return Users;
      case 'relationship_response': return CheckCircle;
      case 'ward_invitation': return UserPlus;
      case 'stewardship_request': return Users;
      default: return InboxIcon;
    }
  }

  function getStatusBadge(status: string): { class: string; label: string } {
    switch (status) {
      case 'pending': return { class: 'badge-warning', label: 'Pending' };
      case 'sent': return { class: 'badge-info', label: 'Sent' };
      case 'delivered': return { class: 'badge-success', label: 'Delivered' };
      case 'read': return { class: 'badge-success', label: 'Read' };
      case 'accepted': return { class: 'badge-success', label: 'Accepted' };
      case 'declined': return { class: 'badge-error', label: 'Declined' };
      case 'failed': return { class: 'badge-error', label: 'Failed' };
      default: return { class: 'badge-info', label: status };
    }
  }

  function formatDate(dateStr: string | undefined): string {
    if (!dateStr) return 'Unknown';
    try {
      return new Date(dateStr).toLocaleString();
    } catch {
      return dateStr;
    }
  }

  async function refresh() {
    await messagesStore.refresh();
    await loadExtras();
  }

  onMount(() => {
    loadExtras();
  });

  $effect(() => {
    if ($user) {
      loadExtras();
    }
  });
</script>

<div class="inbox-page">
  <div class="header">
    <div class="title-section">
      <h1>
        <InboxIcon size={28} />
        Messages
      </h1>
      <p>View your inbox, outbox, and sent messages</p>
    </div>
    <button class="btn btn-secondary" onclick={refresh} disabled={$messagesLoading}>
      <RefreshCw size={18} class={{ spinning: $messagesLoading }} />
      Refresh
    </button>
  </div>

  {#if !$user}
    <div class="alert alert-info">
      <AlertCircle size={18} />
      <span>Login to view your messages.</span>
    </div>
  {:else}
    <div class="tabs-container">
      <div class="tabs">
        <button
          class="tab"
          class:active={activeTab === 'inbox'}
          onclick={() => activeTab = 'inbox'}
        >
          <InboxIcon size={18} />
          Inbox ({$inbox.length})
        </button>
        <button
          class="tab"
          class:active={activeTab === 'outbox'}
          onclick={() => activeTab = 'outbox'}
        >
          <Clock size={18} />
          Outbox ({$outbox.length})
        </button>
        <button
          class="tab"
          class:active={activeTab === 'sent'}
          onclick={() => activeTab = 'sent'}
        >
          <Send size={18} />
          Sent ({$sent.length})
        </button>
      </div>
    </div>

    {#if $messagesLoading}
      <div class="loading">
        <RefreshCw size={24} class="spinning" />
        <span>Loading messages...</span>
      </div>
    {:else}
      {#if activeTab === 'inbox'}
        {#if $inbox.length === 0 && notifications.length === 0 && tasks.length === 0}
          <div class="empty-state">
            <InboxIcon size={48} />
            <h2>Inbox Empty</h2>
            <p>You have no messages in your inbox.</p>
          </div>
        {:else}
          <div class="message-sections">
            {#if tasks.length > 0}
              <div class="section">
                <h3><ClipboardList size={18} /> Tasks ({tasks.length})</h3>
                <div class="message-list">
                  {#each tasks as task (task.id)}
                    <div class="message-card">
                      <div class="message-header">
                        <span class="message-type">
                          <ClipboardList size={16} />
                          {task.properties.task_type || 'Task'}
                        </span>
                        <span class="badge {getStatusBadge(task.properties.status as string).class}">
                          {getStatusBadge(task.properties.status as string).label}
                        </span>
                      </div>
                      <div class="message-title">{task.properties.title || task.name}</div>
                      {#if task.properties.description}
                        <div class="message-body">{task.properties.description}</div>
                      {/if}
                      <div class="message-meta">
                        <code>{task.path}</code>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            {#if notifications.length > 0}
              <div class="section">
                <h3><Bell size={18} /> Notifications ({notifications.length})</h3>
                <div class="message-list">
                  {#each notifications as notif (notif.id)}
                    <div class="message-card">
                      <div class="message-header">
                        <span class="message-type">
                          <Bell size={16} />
                          {notif.properties.notification_type || 'Notification'}
                        </span>
                        <span class="badge {getStatusBadge(notif.properties.status).class}">
                          {getStatusBadge(notif.properties.status).label}
                        </span>
                      </div>
                      <div class="message-title">{notif.properties.title || notif.name}</div>
                      {#if notif.properties.body}
                        <div class="message-body">
                          {typeof notif.properties.body === 'object'
                            ? JSON.stringify(notif.properties.body, null, 2)
                            : notif.properties.body}
                        </div>
                      {/if}
                      <div class="message-meta">
                        <code>{notif.path}</code>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            {#if $inbox.length > 0}
              <div class="section">
                <h3><InboxIcon size={18} /> Messages ({$inbox.length})</h3>
                <div class="message-list">
                  {#each $inbox as msg (msg.id)}
                    {@const Icon = getMessageIcon(msg.properties.message_type)}
                    <div class="message-card">
                      <div class="message-header">
                        <span class="message-type">
                          <svelte:component this={Icon} size={16} />
                          {msg.properties.message_type}
                        </span>
                        <span class="badge {getStatusBadge(msg.properties.status).class}">
                          {getStatusBadge(msg.properties.status).label}
                        </span>
                      </div>
                      <div class="message-title">{msg.properties.subject || msg.name}</div>
                      {#if msg.properties.sender_id}
                        <div class="message-from">
                          From: {msg.properties.sender_display_name || msg.properties.sender_id}
                        </div>
                      {/if}
                      {#if msg.properties.body}
                        <div class="message-body">
                          <pre>{JSON.stringify(msg.properties.body, null, 2)}</pre>
                        </div>
                      {/if}
                      <div class="message-meta">
                        <span>{formatDate(msg.properties.created_at)}</span>
                        <code>{msg.path}</code>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
          </div>
        {/if}
      {/if}

      {#if activeTab === 'outbox'}
        {#if $outbox.length === 0}
          <div class="empty-state">
            <Clock size={48} />
            <h2>Outbox Empty</h2>
            <p>No pending messages. Messages are processed quickly.</p>
          </div>
        {:else}
          <div class="message-list">
            {#each $outbox as msg (msg.id)}
              {@const Icon = getMessageIcon(msg.properties.message_type)}
              <div class="message-card">
                <div class="message-header">
                  <span class="message-type">
                    <svelte:component this={Icon} size={16} />
                    {msg.properties.message_type}
                  </span>
                  <span class="badge {getStatusBadge(msg.properties.status).class}">
                    {getStatusBadge(msg.properties.status).label}
                  </span>
                </div>
                <div class="message-title">{msg.properties.subject || msg.name}</div>
                {#if msg.properties.recipient_id}
                  <div class="message-to">To: {msg.properties.recipient_id}</div>
                {/if}
                {#if msg.properties.body}
                  <div class="message-body">
                    <pre>{JSON.stringify(msg.properties.body, null, 2)}</pre>
                  </div>
                {/if}
                <div class="message-meta">
                  <span>{formatDate(msg.properties.created_at)}</span>
                  <code>{msg.path}</code>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}

      {#if activeTab === 'sent'}
        {#if $sent.length === 0}
          <div class="empty-state">
            <Send size={48} />
            <h2>No Sent Messages</h2>
            <p>Send a message to see it here after it's processed.</p>
            <a href="/send" class="btn btn-primary">Send Message</a>
          </div>
        {:else}
          <div class="message-list">
            {#each $sent as msg (msg.id)}
              {@const Icon = getMessageIcon(msg.properties.message_type)}
              <div class="message-card">
                <div class="message-header">
                  <span class="message-type">
                    <svelte:component this={Icon} size={16} />
                    {msg.properties.message_type}
                  </span>
                  <span class="badge {getStatusBadge(msg.properties.status).class}">
                    {getStatusBadge(msg.properties.status).label}
                  </span>
                </div>
                <div class="message-title">{msg.properties.subject || msg.name}</div>
                {#if msg.properties.recipient_id}
                  <div class="message-to">To: {msg.properties.recipient_id}</div>
                {/if}
                {#if msg.properties.body}
                  <div class="message-body">
                    <pre>{JSON.stringify(msg.properties.body, null, 2)}</pre>
                  </div>
                {/if}
                <div class="message-meta">
                  <span>{formatDate(msg.properties.created_at)}</span>
                  <code>{msg.path}</code>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .inbox-page {
    max-width: 1000px;
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

  .message-sections {
    background: white;
    border-radius: 0 0 0.75rem 0.75rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .section {
    padding: 1.5rem;
    border-bottom: 1px solid #e5e7eb;
  }

  .section:last-child {
    border-bottom: none;
  }

  .section h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1rem;
    font-weight: 600;
    color: #374151;
    margin: 0 0 1rem 0;
  }

  .message-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .message-card {
    background: #f9fafb;
    border-radius: 0.5rem;
    padding: 1rem;
    border: 1px solid #e5e7eb;
  }

  .message-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
  }

  .message-type {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: #6b7280;
    text-transform: uppercase;
  }

  .message-title {
    font-size: 1rem;
    font-weight: 600;
    color: #111827;
    margin-bottom: 0.25rem;
  }

  .message-from,
  .message-to {
    font-size: 0.875rem;
    color: #6b7280;
    margin-bottom: 0.5rem;
  }

  .message-body {
    background: white;
    border-radius: 0.375rem;
    padding: 0.75rem;
    margin-bottom: 0.5rem;
    font-size: 0.875rem;
    color: #374151;
    max-height: 200px;
    overflow: auto;
  }

  .message-body pre {
    margin: 0;
    font-family: monospace;
    font-size: 0.75rem;
    white-space: pre-wrap;
  }

  .message-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.75rem;
    color: #9ca3af;
  }

  .message-meta code {
    font-family: monospace;
    background: white;
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
    max-width: 300px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
