<script lang="ts">
  import { onMount } from 'svelte';
  import { query, ACCESS_CONTROL_WORKSPACE, type UserNode } from '$lib/raisin';
  import { user } from '$lib/stores/auth';
  import { Users, RefreshCw, User, Copy, Check, AlertCircle } from 'lucide-svelte';

  interface UserWithRelations extends UserNode {
    relations?: Array<{ type: string; target_path: string }>;
  }

  let users = $state<UserWithRelations[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let copiedId = $state<string | null>(null);
  let copiedPath = $state<string | null>(null);

  async function loadUsers() {
    loading = true;
    error = null;

    try {
      // Get all users (both external and internal)
      const result = await query<UserNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('/users')
          AND node_type = 'raisin:User'
        ORDER BY path
      `);

      users = result;

      // Load relationships for each user
      for (const u of users) {
        try {
          const relations = await query<{ type: string; target_path: string }>(`
            SELECT * FROM GRAPH_TABLE(
              MATCH (a)-[r]->(b)
              WHERE a.path = '${u.path}'
              COLUMNS (type(r) AS type, b.path AS target_path)
            ) AS g
          `);
          u.relations = relations;
        } catch {
          // Ignore graph query errors
        }
      }
    } catch (err) {
      console.error('[users] Failed to load users:', err);
      error = err instanceof Error ? err.message : 'Failed to load users';
    } finally {
      loading = false;
    }
  }

  function copyId(id: string) {
    navigator.clipboard.writeText(id);
    copiedId = id;
    setTimeout(() => {
      copiedId = null;
    }, 2000);
  }

  function copyPath(path: string) {
    navigator.clipboard.writeText(path);
    copiedPath = path;
    setTimeout(() => {
      copiedPath = null;
    }, 2000);
  }

  function getUserType(path: string): string {
    if (path.includes('/users/internal/')) return 'Internal (Ward)';
    if (path.includes('/users/external/')) return 'External';
    return 'Unknown';
  }

  onMount(() => {
    loadUsers();
  });
</script>

<div class="users-page">
  <div class="header">
    <div class="title-section">
      <h1>
        <Users size={28} />
        Users
      </h1>
      <p>View all users in the system</p>
    </div>
    <button class="btn btn-secondary" onclick={loadUsers} disabled={loading}>
      <RefreshCw size={18} class={{ spinning: loading }} />
      Refresh
    </button>
  </div>

  {#if !$user}
    <div class="alert alert-info">
      <AlertCircle size={18} />
      <span>Login to view users and their IDs.</span>
    </div>
  {/if}

  {#if error}
    <div class="alert alert-error">
      <AlertCircle size={18} />
      <span>{error}</span>
    </div>
  {/if}

  {#if loading}
    <div class="loading">
      <RefreshCw size={24} class="spinning" />
      <span>Loading users...</span>
    </div>
  {:else if users.length === 0}
    <div class="empty-state">
      <Users size={48} />
      <h2>No Users Found</h2>
      <p>Register some test users to get started.</p>
      <a href="/auth?mode=register" class="btn btn-primary">Register User</a>
    </div>
  {:else}
    <div class="users-grid">
      {#each users as u (u.id)}
        {@const isCurrentUser = $user?.id === u.id}
        <div class="user-card" class:current={isCurrentUser}>
          <div class="user-header">
            <div class="user-avatar">
              <User size={24} />
            </div>
            <div class="user-info">
              <div class="user-name">
                {u.properties.display_name || u.name}
                {#if isCurrentUser}
                  <span class="badge badge-info">You</span>
                {/if}
              </div>
              <div class="user-type">{getUserType(u.path)}</div>
            </div>
          </div>

          <div class="user-details">
            <div class="detail path-detail">
              <code>{u.id}</code>
              <button
                class="copy-btn"
                onclick={() => copyId(u.id)}
                title="Copy ID"
              >
                {#if copiedId === u.id}
                  <Check size={14} />
                {:else}
                  <Copy size={14} />
                {/if}
              </button>
            </div>

            <div class="detail path-detail">
              <code>{u.path}</code>
              <button
                class="copy-btn"
                onclick={() => copyPath(u.path)}
                title="Copy path"
              >
                {#if copiedPath === u.path}
                  <Check size={14} />
                {:else}
                  <Copy size={14} />
                {/if}
              </button>
            </div>
          </div>

          {#if u.relations && u.relations.length > 0}
            <div class="user-relations">
              <h4>Relationships</h4>
              {#each u.relations as rel}
                <div class="relation">
                  <span class="relation-type">{rel.type}</span>
                  <span class="relation-target">{rel.target_path}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <div class="info-box">
    <h3>User IDs</h3>
    <p>Messaging uses global user IDs. Use the copy button to copy a user's ID.</p>
    <ul>
      <li>Use IDs for messaging, task assignment, and relationship requests.</li>
      <li>Legacy paths are still shown for stewardship/ward flows.</li>
    </ul>
  </div>
</div>

<style>
  .users-page {
    max-width: 1200px;
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

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.75rem;
    padding: 3rem;
    color: #6b7280;
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
    border-radius: 0.75rem;
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

  .users-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(350px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
  }

  .user-card {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    border: 2px solid transparent;
    transition: border-color 0.2s;
  }

  .user-card.current {
    border-color: #6366f1;
  }

  .user-header {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1rem;
  }

  .user-avatar {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: #f3f4f6;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #6b7280;
  }

  .user-name {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1.125rem;
    font-weight: 600;
    color: #111827;
  }

  .user-type {
    font-size: 0.875rem;
    color: #6b7280;
  }

  .user-details {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .detail {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    color: #6b7280;
  }

  .path-detail {
    background: #f9fafb;
    padding: 0.5rem;
    border-radius: 0.375rem;
    justify-content: space-between;
  }

  .path-detail code {
    font-family: monospace;
    font-size: 0.75rem;
    word-break: break-all;
  }

  .copy-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.25rem;
    border: none;
    background: transparent;
    color: #6b7280;
    cursor: pointer;
    border-radius: 0.25rem;
    transition: all 0.2s;
  }

  .copy-btn:hover {
    background: #e5e7eb;
    color: #374151;
  }

  .user-relations {
    margin-top: 1rem;
    padding-top: 1rem;
    border-top: 1px solid #e5e7eb;
  }

  .user-relations h4 {
    font-size: 0.75rem;
    font-weight: 600;
    color: #6b7280;
    margin: 0 0 0.5rem 0;
    text-transform: uppercase;
  }

  .relation {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    margin-bottom: 0.25rem;
  }

  .relation-type {
    background: #eef2ff;
    color: #4f46e5;
    padding: 0.125rem 0.5rem;
    border-radius: 0.25rem;
    font-weight: 500;
    font-size: 0.75rem;
  }

  .relation-target {
    color: #6b7280;
    font-family: monospace;
    font-size: 0.75rem;
  }

  .info-box {
    background: #f0fdf4;
    border: 1px solid #bbf7d0;
    border-radius: 0.75rem;
    padding: 1.5rem;
  }

  .info-box h3 {
    margin: 0 0 0.5rem 0;
    color: #166534;
    font-size: 1rem;
  }

  .info-box p {
    margin: 0 0 1rem 0;
    color: #166534;
    font-size: 0.875rem;
  }

  .info-box ul {
    margin: 0;
    padding-left: 1.5rem;
    color: #166534;
    font-size: 0.875rem;
  }

  .info-box li {
    margin-bottom: 0.25rem;
  }

  .info-box code {
    background: #dcfce7;
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
    font-family: monospace;
    font-size: 0.8125rem;
  }
</style>
