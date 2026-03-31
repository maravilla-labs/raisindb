<script lang="ts">
  import { onMount } from 'svelte';
  import { user } from '$lib/stores/auth';
  import { messagesStore } from '$lib/stores/messages';
  import { query, ACCESS_CONTROL_WORKSPACE } from '$lib/raisin';
  import {
    Users, UserPlus, Shield, RefreshCw, AlertCircle,
    CheckCircle, Baby, User, ArrowRight, Info
  } from 'lucide-svelte';

  type Tab = 'family' | 'create-ward';

  interface FamilyMember {
    id: string;
    path: string;
    display_name?: string;
    relation_type: string;
  }

  let activeTab = $state<Tab>('family');
  let loading = $state(true);
  let actionLoading = $state(false);
  let actionResult = $state<{ success: boolean; message: string } | null>(null);

  // Data
  let wards = $state<FamilyMember[]>([]);
  let stewards = $state<FamilyMember[]>([]);

  // Create ward form
  let wardForm = $state({
    displayName: '',
    email: '',
    relationType: 'PARENT_OF' as 'PARENT_OF' | 'GUARDIAN_OF'
  });

  function getUserHomePath(): string {
    if (!$user?.home) return '';
    return $user.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');
  }

  async function loadFamily() {
    if (!$user) return;

    try {
      // Load wards (users I am steward of)
      const wardResults = await query<{ id: string; path: string; properties: Record<string, unknown>; relation_type: string }>(`
        SELECT * FROM GRAPH_TABLE(
          MATCH (me)-[r:PARENT_OF|GUARDIAN_OF]->(ward)
          WHERE me.id = '${$user.id}'
          COLUMNS (ward.id AS id, ward.path AS path, ward.properties AS properties, type(r) AS relation_type)
        ) AS g
      `);

      wards = wardResults.map(r => ({
        id: r.id,
        path: r.path,
        display_name: (r.properties as Record<string, unknown>)?.display_name as string,
        relation_type: r.relation_type
      }));
    } catch (err) {
      console.error('[family] Failed to load wards:', err);
      wards = [];
    }

    try {
      // Load stewards (users who are stewards of me)
      const stewardResults = await query<{ id: string; path: string; properties: Record<string, unknown>; relation_type: string }>(`
        SELECT * FROM GRAPH_TABLE(
          MATCH (steward)-[r:PARENT_OF|GUARDIAN_OF]->(me)
          WHERE me.id = '${$user.id}'
          COLUMNS (steward.id AS id, steward.path AS path, steward.properties AS properties, type(r) AS relation_type)
        ) AS g
      `);

      stewards = stewardResults.map(r => ({
        id: r.id,
        path: r.path,
        display_name: (r.properties as Record<string, unknown>)?.display_name as string,
        relation_type: r.relation_type
      }));
    } catch (err) {
      console.error('[family] Failed to load stewards:', err);
      stewards = [];
    }
  }

  async function refresh() {
    loading = true;
    try {
      await loadFamily();
    } finally {
      loading = false;
    }
  }

  async function createWard() {
    if (!$user?.home || !wardForm.displayName.trim()) return;

    actionLoading = true;
    actionResult = null;

    try {
      const homePath = getUserHomePath();
      const messageId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const outboxPath = `${homePath}/outbox/${messageId}`;

      const properties = {
        message_type: 'ward_invitation',
        status: 'pending',
        created_at: new Date().toISOString(),
        sender_id: $user.id,
        steward_path: homePath,
        ward_display_name: wardForm.displayName.trim(),
        ward_email: wardForm.email.trim() || null,
        relation_type: wardForm.relationType
      };

      await query(`
        INSERT INTO '${ACCESS_CONTROL_WORKSPACE}' (path, node_type, properties)
        VALUES ($1, 'raisin:Message', $2::jsonb)
      `, [outboxPath, JSON.stringify(properties)]);

      actionResult = { success: true, message: `Ward "${wardForm.displayName}" creation request sent!` };

      // Reset form
      wardForm.displayName = '';
      wardForm.email = '';

      await refresh();
      await messagesStore.refresh();
    } catch (err) {
      console.error('[family] Failed to create ward:', err);
      actionResult = { success: false, message: err instanceof Error ? err.message : 'Failed to create ward' };
    } finally {
      actionLoading = false;
    }
  }

  async function requestStewardship(wardPath: string) {
    if (!$user?.home) return;

    actionLoading = true;
    actionResult = null;

    try {
      const homePath = getUserHomePath();
      const messageId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const outboxPath = `${homePath}/outbox/${messageId}`;

      const properties = {
        message_type: 'stewardship_request',
        status: 'pending',
        created_at: new Date().toISOString(),
        sender_id: $user.id,
        body: {
          steward_path: homePath,
          ward_path: wardPath,
          delegation_type: 'general',
          scope: ['read_profile', 'manage_tasks'],
          message: 'I would like to request stewardship access.'
        }
      };

      await query(`
        INSERT INTO '${ACCESS_CONTROL_WORKSPACE}' (path, node_type, properties)
        VALUES ($1, 'raisin:Message', $2::jsonb)
      `, [outboxPath, JSON.stringify(properties)]);

      actionResult = { success: true, message: 'Stewardship request sent!' };
      await messagesStore.refresh();
    } catch (err) {
      console.error('[family] Failed to request stewardship:', err);
      actionResult = { success: false, message: err instanceof Error ? err.message : 'Failed to send request' };
    } finally {
      actionLoading = false;
    }
  }

  function getRelationLabel(relationType: string): string {
    switch (relationType) {
      case 'PARENT_OF': return 'Parent';
      case 'GUARDIAN_OF': return 'Guardian';
      default: return relationType;
    }
  }

  function getWardRelationLabel(relationType: string): string {
    switch (relationType) {
      case 'PARENT_OF': return 'Child';
      case 'GUARDIAN_OF': return 'Ward';
      default: return 'Dependant';
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

<div class="family-page">
  <div class="header">
    <div class="title-section">
      <h1>
        <Users size={28} />
        Family
      </h1>
      <p>Manage wards and stewardship relationships</p>
    </div>
    <button class="btn btn-secondary" onclick={refresh} disabled={loading}>
      <span class:spinning={loading}><RefreshCw size={18} /></span>
      Refresh
    </button>
  </div>

  {#if !$user}
    <div class="alert alert-info">
      <AlertCircle size={18} />
      <span>Login to manage your family.</span>
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
          class:active={activeTab === 'family'}
          onclick={() => activeTab = 'family'}
        >
          <Users size={18} />
          My Family
        </button>
        <button
          class="tab"
          class:active={activeTab === 'create-ward'}
          onclick={() => activeTab = 'create-ward'}
        >
          <UserPlus size={18} />
          Create Ward
        </button>
      </div>
    </div>

    {#if loading}
      <div class="loading">
        <span class="spinning"><RefreshCw size={24} /></span>
        <span>Loading family...</span>
      </div>
    {:else}
      {#if activeTab === 'family'}
        <div class="content-card">
          {#if wards.length === 0 && stewards.length === 0}
            <div class="empty-state-inline">
              <Users size={48} />
              <h3>No Family Relationships</h3>
              <p>Create a ward to manage a dependant account, or accept stewardship requests from others.</p>
              <button class="btn btn-primary" onclick={() => activeTab = 'create-ward'}>
                <UserPlus size={18} />
                Create Ward
              </button>
            </div>
          {:else}
            {#if wards.length > 0}
              <div class="family-section">
                <h3>
                  <Baby size={18} />
                  My Wards ({wards.length})
                </h3>
                <p class="section-desc">Users you are a steward of (you can act on their behalf)</p>
                <div class="family-list">
                  {#each wards as ward (ward.id)}
                    <div class="family-card">
                      <div class="family-avatar ward-avatar">
                        <Baby size={24} />
                      </div>
                      <div class="family-info">
                        <div class="family-name">{ward.display_name || ward.path.split('/').pop()}</div>
                        <div class="family-meta">
                          <span class="badge badge-info">{getWardRelationLabel(ward.relation_type)}</span>
                          <code>{ward.id}</code>
                        </div>
                      </div>
                      <button class="btn btn-sm btn-secondary" title="Switch to ward account (not implemented)" disabled>
                        <ArrowRight size={16} />
                        Switch
                      </button>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            {#if stewards.length > 0}
              <div class="family-section">
                <h3>
                  <Shield size={18} />
                  My Stewards ({stewards.length})
                </h3>
                <p class="section-desc">Users who can act on your behalf</p>
                <div class="family-list">
                  {#each stewards as steward (steward.id)}
                    <div class="family-card">
                      <div class="family-avatar steward-avatar">
                        <Shield size={24} />
                      </div>
                      <div class="family-info">
                        <div class="family-name">{steward.display_name || steward.path.split('/').pop()}</div>
                        <div class="family-meta">
                          <span class="badge badge-success">{getRelationLabel(steward.relation_type)}</span>
                          <code>{steward.id}</code>
                        </div>
                      </div>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}
          {/if}
        </div>
      {/if}

      {#if activeTab === 'create-ward'}
        <div class="content-card">
          <div class="form-card">
            <h3>
              <UserPlus size={20} />
              Create Ward Account
            </h3>
            <p class="form-description">
              Create a new user account that you can manage. This is useful for children or dependants who need accounts but require oversight.
            </p>

            <div class="form-group">
              <label class="form-label" for="ward-name">Display Name *</label>
              <input
                type="text"
                id="ward-name"
                class="form-input"
                bind:value={wardForm.displayName}
                placeholder="e.g., Tommy"
              />
              <span class="form-hint">The name that will be displayed for this ward</span>
            </div>

            <div class="form-group">
              <label class="form-label" for="ward-email">Email (optional)</label>
              <input
                type="email"
                id="ward-email"
                class="form-input"
                bind:value={wardForm.email}
                placeholder="tommy@example.com"
              />
              <span class="form-hint">Optional email for the ward (for future notifications)</span>
            </div>

            <div class="form-group">
              <label class="form-label" for="ward-relation">Relation Type</label>
              <select id="ward-relation" class="form-input form-select" bind:value={wardForm.relationType}>
                <option value="PARENT_OF">Parent (PARENT_OF)</option>
                <option value="GUARDIAN_OF">Guardian (GUARDIAN_OF)</option>
              </select>
              <span class="form-hint">Your relationship to this ward</span>
            </div>

            <div class="alert alert-info">
              <Info size={16} />
              <div>
                <p><strong>What happens when you create a ward:</strong></p>
                <ul>
                  <li>A new internal user will be created at <code>/users/internal/[id]</code></li>
                  <li>You will be set as the {wardForm.relationType} of this user</li>
                  <li>You can act on behalf of this ward for messaging and tasks</li>
                </ul>
              </div>
            </div>

            <div class="form-actions">
              <button
                class="btn btn-primary btn-lg"
                onclick={createWard}
                disabled={actionLoading || !wardForm.displayName.trim()}
              >
                {#if actionLoading}
                  <span class="spinning"><RefreshCw size={18} /></span>
                  Creating...
                {:else}
                  <UserPlus size={18} />
                  Create Ward
                {/if}
              </button>
            </div>
          </div>
        </div>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .family-page {
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

  .empty-state-inline {
    text-align: center;
    padding: 3rem;
    color: #6b7280;
  }

  .empty-state-inline :global(svg) {
    color: #9ca3af;
    margin-bottom: 1rem;
  }

  .empty-state-inline h3 {
    margin: 0 0 0.5rem 0;
    color: #374151;
  }

  .empty-state-inline p {
    margin: 0 0 1.5rem 0;
  }

  .family-section {
    margin-bottom: 2rem;
  }

  .family-section:last-child {
    margin-bottom: 0;
  }

  .family-section h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1rem;
    font-weight: 600;
    color: #374151;
    margin: 0 0 0.25rem 0;
  }

  .section-desc {
    font-size: 0.875rem;
    color: #6b7280;
    margin: 0 0 1rem 0;
  }

  .family-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .family-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1rem;
    background: #f9fafb;
    border-radius: 0.5rem;
    border: 1px solid #e5e7eb;
  }

  .family-avatar {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .ward-avatar {
    background: #fef3c7;
    color: #d97706;
  }

  .steward-avatar {
    background: #dcfce7;
    color: #16a34a;
  }

  .family-info {
    flex: 1;
    min-width: 0;
  }

  .family-name {
    font-weight: 600;
    color: #111827;
    margin-bottom: 0.25rem;
  }

  .family-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
  }

  .family-meta code {
    color: #6b7280;
    font-family: monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Form styles */
  .form-card {
    max-width: 600px;
  }

  .form-card h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1.25rem;
    font-weight: 600;
    color: #111827;
    margin: 0 0 0.5rem 0;
  }

  .form-description {
    color: #6b7280;
    font-size: 0.9375rem;
    margin: 0 0 1.5rem 0;
    line-height: 1.5;
  }

  .form-group {
    margin-bottom: 1.25rem;
  }

  .form-label {
    display: block;
    font-weight: 500;
    color: #374151;
    margin-bottom: 0.5rem;
    font-size: 0.9375rem;
  }

  .form-input {
    width: 100%;
    padding: 0.75rem 1rem;
    border: 1px solid #d1d5db;
    border-radius: 0.5rem;
    font-size: 0.9375rem;
    transition: border-color 0.2s, box-shadow 0.2s;
  }

  .form-input:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .form-select {
    appearance: none;
    background-image: url("data:image/svg+xml,%3csvg xmlns='http://www.w3.org/2000/svg' fill='none' viewBox='0 0 20 20'%3e%3cpath stroke='%236b7280' stroke-linecap='round' stroke-linejoin='round' stroke-width='1.5' d='M6 8l4 4 4-4'/%3e%3c/svg%3e");
    background-position: right 0.75rem center;
    background-repeat: no-repeat;
    background-size: 1.25em 1.25em;
    padding-right: 2.5rem;
  }

  .form-hint {
    display: block;
    font-size: 0.75rem;
    color: #9ca3af;
    margin-top: 0.375rem;
  }

  .form-actions {
    margin-top: 1.5rem;
  }

  .btn-lg {
    padding: 0.875rem 1.5rem;
    font-size: 1rem;
  }

  /* Alert styles */
  .alert {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 1rem;
    border-radius: 0.5rem;
    margin-bottom: 1rem;
  }

  .alert :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
  }

  .alert-info {
    background: #eff6ff;
    color: #1e40af;
    border: 1px solid #bfdbfe;
  }

  .alert-info p {
    margin: 0 0 0.5rem 0;
  }

  .alert-info ul {
    margin: 0;
    padding-left: 1.25rem;
    font-size: 0.875rem;
  }

  .alert-info li {
    margin-bottom: 0.25rem;
  }

  .alert-info code {
    background: rgba(255, 255, 255, 0.5);
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
    font-family: monospace;
    font-size: 0.8125rem;
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

  /* Badge styles */
  .badge {
    display: inline-flex;
    align-items: center;
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
  }

  .badge-success {
    background: #dcfce7;
    color: #166534;
  }

  .badge-info {
    background: #dbeafe;
    color: #1e40af;
  }
</style>
