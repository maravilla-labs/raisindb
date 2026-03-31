<script lang="ts">
  import { X, Send, AlertCircle, CheckCircle, Bot } from 'lucide-svelte';
  import { getDatabase, query } from '$lib/raisin';
  import { user } from '$lib/stores/auth';
  import { get } from 'svelte/store';
  import { onMount } from 'svelte';

  interface Props {
    onClose: () => void;
    onConversationCreated?: (conversationPath: string) => void;
    preselectedId?: string;
  }

  let { onClose, onConversationCreated, preselectedId = '' }: Props = $props();

  interface FriendItem {
    id: string;
    path: string;
    properties: { email?: string; display_name?: string };
  }

  interface AgentItem {
    path: string;
    userId: string;
    displayName: string;
  }

  // Form state
  let friends = $state<FriendItem[]>([]);
  let agents = $state<AgentItem[]>([]);
  let selectedRecipientId = $state(preselectedId);
  let selectedRecipientType = $state<'friend' | 'agent'>('friend');
  let content = $state('');
  let sending = $state(false);
  let error = $state<string | null>(null);
  let success = $state(false);
  let loading = $state(true);

  const selectedFriend = $derived(
    selectedRecipientType === 'friend'
      ? friends.find(f => f.id === selectedRecipientId)
      : undefined
  );
  const selectedAgent = $derived(
    selectedRecipientType === 'agent'
      ? agents.find(a => a.userId === selectedRecipientId)
      : undefined
  );
  const hasRecipient = $derived(!!selectedFriend || !!selectedAgent);

  onMount(async () => {
    const [friendList, agentList] = await Promise.all([loadFriends(), loadAgents()]);
    friends = friendList;
    agents = agentList;
    if (preselectedId && preselectedId.startsWith('agent:')) {
      selectedRecipientType = 'agent';
    }
    loading = false;
  });

  async function loadFriends(): Promise<FriendItem[]> {
    const currentUser = get(user);
    if (!currentUser?.home) return [];
    const homePath = currentUser.home.replace('/raisin:access_control', '');
    try {
      const sql = `SELECT * FROM GRAPH_TABLE(MATCH (me)-[:FRIENDS_WITH]->(friend) WHERE me.path = '${homePath}' COLUMNS (friend.id AS id, friend.path AS path, friend.properties AS properties)) AS g`;
      return (await query<FriendItem>(sql)) || [];
    } catch { return []; }
  }

  async function loadAgents(): Promise<AgentItem[]> {
    try {
      const rows = await query<{ path: string; name: string; properties: Record<string, unknown> }>(`
        SELECT path, name, properties FROM 'functions'
        WHERE node_type = 'raisin:AIAgent' AND CHILD_OF('/agents')
        ORDER BY name ASC
      `);
      return rows.map(row => ({
        path: row.path,
        userId: 'agent:' + row.name,
        displayName: row.name.replace(/-/g, ' ').replace(/\b\w/g, c => c.toUpperCase()),
      }));
    } catch { return []; }
  }

  function selectRecipient(type: 'friend' | 'agent', id: string) {
    selectedRecipientType = type;
    selectedRecipientId = id;
  }

  async function handleSubmit(e: Event) {
    e.preventDefault();

    if (!hasRecipient) {
      error = 'Please select a recipient';
      return;
    }

    if (!content.trim()) {
      error = 'Please enter a message';
      return;
    }

    sending = true;
    error = null;

    try {
      const db = await getDatabase();
      const participant = selectedRecipientType === 'agent'
        ? selectedAgent!.userId
        : selectedFriend!.id;

      // Create conversation via SDK
      const convo = await db.conversations.create({ participant });

      // Send the first message
      await db.conversations.createUserMessage(convo.conversationPath, content.trim());

      success = true;
      setTimeout(() => {
        onConversationCreated?.(convo.conversationPath);
        onClose();
      }, 800);
    } catch (err) {
      error = err instanceof Error ? err.message : 'Failed to create conversation';
    }

    sending = false;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      onClose();
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="modal-overlay" onclick={onClose} role="dialog" aria-modal="true">
  <div class="modal-content" onclick={(e) => e.stopPropagation()}>
    <div class="modal-header">
      <h2>New Chat</h2>
      <button class="btn-close" onclick={onClose} aria-label="Close">
        <X size={20} />
      </button>
    </div>

    {#if success}
      <div class="success-state">
        <CheckCircle size={48} />
        <h3>Chat Created!</h3>
        <p>Opening conversation...</p>
      </div>
    {:else}
      <form class="modal-body" onsubmit={handleSubmit}>
        {#if error}
          <div class="error-message">
            <AlertCircle size={16} />
            {error}
          </div>
        {/if}

        <div class="form-field">
          <label>To</label>
          {#if loading}
            <div class="loading-placeholder">Loading recipients...</div>
          {:else if friends.length === 0 && agents.length === 0}
            <div class="no-friends">
              <p>No recipients available.</p>
              <a href="/friends">Add friends first</a>
            </div>
          {:else}
            <div class="recipient-list">
              {#if agents.length > 0}
                <div class="recipient-section">
                  <span class="section-label"><Bot size={14} /> AI Agents</span>
                  {#each agents as agent}
                    <button
                      type="button"
                      class="recipient-option"
                      class:selected={selectedRecipientType === 'agent' && selectedRecipientId === agent.userId}
                      onclick={() => selectRecipient('agent', agent.userId)}
                      disabled={sending}
                    >
                      <span class="agent-badge">AI</span>
                      {agent.displayName}
                    </button>
                  {/each}
                </div>
              {/if}
              {#if friends.length > 0}
                <div class="recipient-section">
                  <span class="section-label">Friends</span>
                  {#each friends as friend}
                    <button
                      type="button"
                      class="recipient-option"
                      class:selected={selectedRecipientType === 'friend' && selectedRecipientId === friend.id}
                      onclick={() => selectRecipient('friend', friend.id)}
                      disabled={sending}
                    >
                      {friend.properties?.display_name || 'Friend'}
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </div>

        <div class="form-field">
          <label for="content">Message</label>
          <textarea
            id="content"
            bind:value={content}
            placeholder="Type your message..."
            rows="3"
            disabled={sending}
          ></textarea>
        </div>

        <div class="modal-footer">
          <button type="button" class="btn-cancel" onclick={onClose} disabled={sending}>
            Cancel
          </button>
          <button
            type="submit"
            class="btn-send"
            disabled={sending || !hasRecipient || !content.trim()}
          >
            <Send size={16} />
            {sending ? 'Creating...' : 'Start Chat'}
          </button>
        </div>
      </form>
    {/if}
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
    padding: 1rem;
  }

  .modal-content {
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    width: 100%;
    max-width: 480px;
    max-height: 90vh;
    overflow: auto;
    box-shadow: var(--shadow-lg);
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    border-bottom: 1px solid var(--color-border);
  }

  .modal-header h2 {
    font-size: 1.125rem;
    font-weight: 600;
    font-family: var(--font-display);
    margin: 0;
    color: var(--color-text-heading);
  }

  .btn-close {
    padding: 0.375rem;
    background: none;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .btn-close:hover {
    color: var(--color-text);
    background: var(--color-surface);
  }

  .modal-body {
    padding: 1.5rem;
  }

  .success-state {
    padding: 3rem 1.5rem;
    text-align: center;
  }

  .success-state :global(svg) {
    color: var(--color-success);
    margin-bottom: 1rem;
  }

  .success-state h3 {
    font-size: 1.25rem;
    font-weight: 600;
    font-family: var(--font-display);
    color: var(--color-text-heading);
    margin: 0 0 0.5rem;
  }

  .success-state p {
    color: var(--color-text-secondary);
    margin: 0;
  }

  .error-message {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: var(--color-error-muted);
    color: var(--color-error);
    border: 1px solid rgba(239, 68, 68, 0.2);
    padding: 0.75rem 1rem;
    border-radius: var(--radius-sm);
    font-size: 0.875rem;
    margin-bottom: 1rem;
  }

  .form-field {
    margin-bottom: 1rem;
  }

  .form-field label {
    display: block;
    font-size: 0.875rem;
    font-weight: 500;
    font-family: var(--font-body);
    color: var(--color-text-secondary);
    margin-bottom: 0.375rem;
  }

  .form-field textarea {
    width: 100%;
    padding: 0.625rem 0.875rem;
    background: var(--color-surface);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.875rem;
    font-family: var(--font-body);
    transition: border-color 0.2s, box-shadow 0.2s;
    resize: vertical;
    min-height: 80px;
  }

  .form-field textarea::placeholder {
    color: var(--color-text-muted);
  }

  .form-field textarea:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px var(--color-accent-glow);
  }

  .form-field textarea:disabled {
    background: var(--color-bg-elevated);
    opacity: 0.5;
    cursor: not-allowed;
  }

  .loading-placeholder {
    padding: 0.625rem 0.875rem;
    background: var(--color-bg-elevated);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: 0.875rem;
  }

  .no-friends {
    padding: 1rem;
    background: var(--color-warning-muted);
    border: 1px solid rgba(245, 158, 11, 0.2);
    border-radius: var(--radius-sm);
    text-align: center;
  }

  .no-friends p {
    color: var(--color-warning);
    margin: 0 0 0.5rem;
    font-size: 0.875rem;
  }

  .no-friends a {
    color: var(--color-accent);
    font-size: 0.875rem;
    font-weight: 500;
  }

  .recipient-list {
    max-height: 200px;
    overflow-y: auto;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
  }

  .recipient-section {
    padding: 0.25rem 0;
  }

  .recipient-section + .recipient-section {
    border-top: 1px solid var(--color-border-subtle);
  }

  .section-label {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.75rem;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .recipient-option {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.5rem 0.75rem;
    background: none;
    border: none;
    font-size: 0.875rem;
    color: var(--color-text);
    cursor: pointer;
    text-align: left;
    transition: background 0.15s;
    font-family: var(--font-body);
  }

  .recipient-option:hover:not(:disabled) {
    background: var(--color-surface);
  }

  .recipient-option.selected {
    background: var(--color-accent-muted);
    color: var(--color-accent);
    font-weight: 500;
  }

  .recipient-option:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .agent-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    background: linear-gradient(135deg, #8b5cf6, #6366f1);
    color: white;
    font-size: 0.625rem;
    font-weight: 700;
    border-radius: 6px;
    flex-shrink: 0;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid var(--color-border);
  }

  .btn-cancel {
    padding: 0.625rem 1rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font-weight: 500;
    font-family: var(--font-body);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-cancel:hover:not(:disabled) {
    background: var(--color-bg-card);
  }

  .btn-send {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1.25rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-sm);
    font-weight: 500;
    font-family: var(--font-body);
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-send:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }

  .btn-cancel:disabled,
  .btn-send:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
