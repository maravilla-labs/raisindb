<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { messagesStore } from '$lib/stores/messages';
  import { query, ACCESS_CONTROL_WORKSPACE } from '$lib/raisin';
  import {
    Send, Bell, ClipboardList, Users, UserPlus, Shield,
    CheckCircle, AlertCircle, Info
  } from 'lucide-svelte';

  type MessageType =
    | 'system_notification'
    | 'task_assignment'
    | 'relationship_request'
    | 'relationship_response'
    | 'ward_invitation'
    | 'stewardship_request';

  const messageTypes: Array<{ type: MessageType; label: string; icon: typeof Bell; description: string }> = [
    {
      type: 'system_notification',
      label: 'System Notification',
      icon: Bell,
      description: 'Send a simple notification to a user'
    },
    {
      type: 'task_assignment',
      label: 'Task Assignment',
      icon: ClipboardList,
      description: 'Assign a task to a user'
    },
    {
      type: 'relationship_request',
      label: 'Relationship Request',
      icon: Users,
      description: 'Request a relationship (PARENT_OF, MANAGER_OF, etc.)'
    },
    {
      type: 'relationship_response',
      label: 'Relationship Response',
      icon: CheckCircle,
      description: 'Accept or reject a relationship request'
    },
    {
      type: 'ward_invitation',
      label: 'Ward Invitation',
      icon: UserPlus,
      description: 'Create a ward account'
    },
    {
      type: 'stewardship_request',
      label: 'Stewardship Request',
      icon: Shield,
      description: 'Request stewardship of another user'
    }
  ];

  let activeType = $state<MessageType>('system_notification');
  let sending = $state(false);
  let result = $state<{ success: boolean; message: string; path?: string } | null>(null);

  // Form state for each message type
  let notificationForm = $state({
    recipientId: '',
    subject: 'Test Notification',
    notificationType: 'info',
    title: 'Notification Title',
    body: 'This is a test notification body.',
    priority: 3
  });

  let taskForm = $state({
    assigneeId: '',
    taskType: 'approval',
    title: 'Review Document',
    description: 'Please review and approve this document.',
    priority: 3,
    options: 'Approve,Reject',
    dueInSeconds: 86400
  });

  let relationshipRequestForm = $state({
    recipientId: '',
    relationType: 'MANAGER_OF',
    message: 'I would like to establish a relationship with you.'
  });

  let relationshipResponseForm = $state({
    accepted: true,
    originalRequestId: '',
    rejectionReason: ''
  });

  let wardInvitationForm = $state({
    wardDisplayName: 'Tommy',
    wardEmail: '',
    relationType: 'PARENT_OF'
  });

  let stewardshipRequestForm = $state({
    wardPath: '',
    delegationType: 'general',
    scope: 'read_profile',
    message: 'I would like to request stewardship access.'
  });

  function getUserHomePath(): string {
    if (!$user?.home) return '';
    return $user.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');
  }

  async function sendMessage() {
    if (!$user?.home) {
      result = { success: false, message: 'You must be logged in to send messages' };
      return;
    }

    sending = true;
    result = null;

    try {
      const homePath = getUserHomePath();
      const messageId = `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const outboxPath = `${homePath}/outbox/${messageId}`;

      let properties: Record<string, unknown> = {
        message_type: activeType,
        status: 'pending',
        created_at: new Date().toISOString(),
        sender_id: $user.id
      };

      switch (activeType) {
        case 'system_notification':
          properties = {
            ...properties,
            subject: notificationForm.subject,
            recipient_id: notificationForm.recipientId,
            body: {
              notification_type: notificationForm.notificationType,
              title: notificationForm.title,
              body: notificationForm.body,
              priority: notificationForm.priority
            }
          };
          break;

        case 'task_assignment':
          properties = {
            ...properties,
            subject: taskForm.title,
            body: {
              assignee_id: taskForm.assigneeId,
              task_type: taskForm.taskType,
              title: taskForm.title,
              description: taskForm.description,
              priority: taskForm.priority,
              options: taskForm.options.split(',').map(o => o.trim()),
              due_in_seconds: taskForm.dueInSeconds
            }
          };
          break;

        case 'relationship_request':
          properties = {
            ...properties,
            recipient_id: relationshipRequestForm.recipientId,
            relation_type: relationshipRequestForm.relationType,
            message: relationshipRequestForm.message
          };
          break;

        case 'relationship_response':
          properties = {
            ...properties,
            accepted: relationshipResponseForm.accepted,
            original_request_id: relationshipResponseForm.originalRequestId,
            rejection_reason: relationshipResponseForm.rejectionReason
          };
          break;

        case 'ward_invitation':
          properties = {
            ...properties,
            steward_path: homePath,
            ward_display_name: wardInvitationForm.wardDisplayName,
            ward_email: wardInvitationForm.wardEmail || null,
            relation_type: wardInvitationForm.relationType
          };
          break;

        case 'stewardship_request':
          properties = {
            ...properties,
            body: {
              steward_path: homePath,
              ward_path: stewardshipRequestForm.wardPath,
              delegation_type: stewardshipRequestForm.delegationType,
              scope: stewardshipRequestForm.scope.split(',').map(s => s.trim()),
              message: stewardshipRequestForm.message
            }
          };
          break;
      }

      // Insert the message into the outbox
      await query(`
        INSERT INTO '${ACCESS_CONTROL_WORKSPACE}' (path, node_type, properties)
        VALUES ($1, 'raisin:Message', $2::jsonb)
      `, [outboxPath, JSON.stringify(properties)]);

      result = {
        success: true,
        message: `Message sent! Check your outbox for status.`,
        path: outboxPath
      };

      // Refresh messages
      await messagesStore.refresh();

    } catch (err) {
      console.error('[send] Failed to send message:', err);
      result = {
        success: false,
        message: err instanceof Error ? err.message : 'Failed to send message'
      };
    } finally {
      sending = false;
    }
  }
</script>

<div class="send-page">
  <div class="header">
    <div class="title-section">
      <h1>
        <Send size={28} />
        Send Message
      </h1>
      <p>Create and send messages to test the messaging system</p>
    </div>
  </div>

  {#if !$user}
    <div class="alert alert-info">
      <AlertCircle size={18} />
      <span>Login to send messages.</span>
    </div>
  {:else}
    <div class="message-type-selector">
      <h2>Select Message Type</h2>
      <div class="type-grid">
        {#each messageTypes as mt}
          <button
            class="type-card"
            class:active={activeType === mt.type}
            onclick={() => { activeType = mt.type; result = null; }}
          >
            <svelte:component this={mt.icon} size={24} />
            <span class="type-label">{mt.label}</span>
            <span class="type-desc">{mt.description}</span>
          </button>
        {/each}
      </div>
    </div>

    {#if result}
      <div class="alert" class:alert-success={result.success} class:alert-error={!result.success}>
        {#if result.success}
          <CheckCircle size={18} />
        {:else}
          <AlertCircle size={18} />
        {/if}
        <div>
          <span>{result.message}</span>
          {#if result.path}
            <code class="result-path">{result.path}</code>
          {/if}
        </div>
      </div>
    {/if}

    <div class="form-container">
      {#if activeType === 'system_notification'}
        <div class="form-card">
          <h3><Bell size={20} /> System Notification</h3>
          <p class="form-description">Send a simple notification to another user.</p>

          <div class="form-group">
            <label class="form-label" for="notif-recipient">Recipient ID *</label>
            <input
              type="text"
              id="notif-recipient"
              class="form-input"
              bind:value={notificationForm.recipientId}
              placeholder="User ID (UUID)"
            />
            <span class="form-hint">Global user ID of the recipient</span>
          </div>

          <div class="form-group">
            <label class="form-label" for="notif-subject">Subject</label>
            <input
              type="text"
              id="notif-subject"
              class="form-input"
              bind:value={notificationForm.subject}
            />
          </div>

          <div class="form-row">
            <div class="form-group">
              <label class="form-label" for="notif-type">Notification Type</label>
              <select id="notif-type" class="form-input form-select" bind:value={notificationForm.notificationType}>
                <option value="info">Info</option>
                <option value="success">Success</option>
                <option value="warning">Warning</option>
                <option value="error">Error</option>
              </select>
            </div>

            <div class="form-group">
              <label class="form-label" for="notif-priority">Priority</label>
              <select id="notif-priority" class="form-input form-select" bind:value={notificationForm.priority}>
                <option value={1}>1 - Low</option>
                <option value={2}>2</option>
                <option value={3}>3 - Normal</option>
                <option value={4}>4</option>
                <option value={5}>5 - High</option>
              </select>
            </div>
          </div>

          <div class="form-group">
            <label class="form-label" for="notif-title">Title</label>
            <input
              type="text"
              id="notif-title"
              class="form-input"
              bind:value={notificationForm.title}
            />
          </div>

          <div class="form-group">
            <label class="form-label" for="notif-body">Body</label>
            <textarea
              id="notif-body"
              class="form-input form-textarea"
              bind:value={notificationForm.body}
            ></textarea>
          </div>
        </div>
      {/if}

      {#if activeType === 'task_assignment'}
        <div class="form-card">
          <h3><ClipboardList size={20} /> Task Assignment</h3>
          <p class="form-description">Assign a task to a user. Creates InboxTask + Notification.</p>

          <div class="form-group">
            <label class="form-label" for="task-assignee">Assignee ID *</label>
            <input
              type="text"
              id="task-assignee"
              class="form-input"
              bind:value={taskForm.assigneeId}
              placeholder="User ID (UUID)"
            />
          </div>

          <div class="form-row">
            <div class="form-group">
              <label class="form-label" for="task-type">Task Type</label>
              <select id="task-type" class="form-input form-select" bind:value={taskForm.taskType}>
                <option value="approval">Approval</option>
                <option value="input">Input</option>
                <option value="review">Review</option>
                <option value="action">Action</option>
              </select>
            </div>

            <div class="form-group">
              <label class="form-label" for="task-priority">Priority</label>
              <select id="task-priority" class="form-input form-select" bind:value={taskForm.priority}>
                <option value={1}>1 - Low</option>
                <option value={3}>3 - Normal</option>
                <option value={5}>5 - High</option>
              </select>
            </div>
          </div>

          <div class="form-group">
            <label class="form-label" for="task-title">Title</label>
            <input
              type="text"
              id="task-title"
              class="form-input"
              bind:value={taskForm.title}
            />
          </div>

          <div class="form-group">
            <label class="form-label" for="task-desc">Description</label>
            <textarea
              id="task-desc"
              class="form-input form-textarea"
              bind:value={taskForm.description}
            ></textarea>
          </div>

          <div class="form-group">
            <label class="form-label" for="task-options">Options (comma-separated)</label>
            <input
              type="text"
              id="task-options"
              class="form-input"
              bind:value={taskForm.options}
              placeholder="Approve,Reject"
            />
            <span class="form-hint">For approval type tasks</span>
          </div>
        </div>
      {/if}

      {#if activeType === 'relationship_request'}
        <div class="form-card">
          <h3><Users size={20} /> Relationship Request</h3>
          <p class="form-description">Request a relationship with another user.</p>

          <div class="form-group">
            <label class="form-label" for="rel-recipient">Recipient ID *</label>
            <input
              type="text"
              id="rel-recipient"
              class="form-input"
              bind:value={relationshipRequestForm.recipientId}
              placeholder="User ID (UUID)"
            />
          </div>

          <div class="form-group">
            <label class="form-label" for="rel-type">Relation Type</label>
            <select id="rel-type" class="form-input form-select" bind:value={relationshipRequestForm.relationType}>
              <option value="PARENT_OF">PARENT_OF</option>
              <option value="GUARDIAN_OF">GUARDIAN_OF</option>
              <option value="MANAGER_OF">MANAGER_OF</option>
            </select>
          </div>

          <div class="form-group">
            <label class="form-label" for="rel-message">Message</label>
            <textarea
              id="rel-message"
              class="form-input form-textarea"
              bind:value={relationshipRequestForm.message}
            ></textarea>
          </div>

          <div class="alert alert-info">
            <Info size={16} />
            <span>You ({$user.id}) will become the {relationshipRequestForm.relationType} the recipient.</span>
          </div>
        </div>
      {/if}

      {#if activeType === 'relationship_response'}
        <div class="form-card">
          <h3><CheckCircle size={20} /> Relationship Response</h3>
          <p class="form-description">Accept or reject a relationship request.</p>

          <div class="form-group">
            <label class="form-label" for="resp-request-id">Original Request ID *</label>
            <input
              type="text"
              id="resp-request-id"
              class="form-input"
              bind:value={relationshipResponseForm.originalRequestId}
              placeholder="UUID of the original request"
            />
          </div>

          <div class="form-group">
            <label class="form-label">Response</label>
            <div class="radio-group">
              <label class="radio-label">
                <input type="radio" bind:group={relationshipResponseForm.accepted} value={true} />
                <CheckCircle size={16} />
                Accept
              </label>
              <label class="radio-label">
                <input type="radio" bind:group={relationshipResponseForm.accepted} value={false} />
                <AlertCircle size={16} />
                Reject
              </label>
            </div>
          </div>

          {#if !relationshipResponseForm.accepted}
            <div class="form-group">
              <label class="form-label" for="resp-reason">Rejection Reason</label>
              <textarea
                id="resp-reason"
                class="form-input form-textarea"
                bind:value={relationshipResponseForm.rejectionReason}
              ></textarea>
            </div>
          {/if}
        </div>
      {/if}

      {#if activeType === 'ward_invitation'}
        <div class="form-card">
          <h3><UserPlus size={20} /> Ward Invitation</h3>
          <p class="form-description">Create a ward account that you can manage.</p>

          <div class="form-group">
            <label class="form-label" for="ward-name">Ward Display Name *</label>
            <input
              type="text"
              id="ward-name"
              class="form-input"
              bind:value={wardInvitationForm.wardDisplayName}
              placeholder="e.g., Tommy"
            />
          </div>

          <div class="form-group">
            <label class="form-label" for="ward-email">Ward Email (optional)</label>
            <input
              type="email"
              id="ward-email"
              class="form-input"
              bind:value={wardInvitationForm.wardEmail}
              placeholder="tommy@example.com"
            />
          </div>

          <div class="form-group">
            <label class="form-label" for="ward-rel">Relation Type</label>
            <select id="ward-rel" class="form-input form-select" bind:value={wardInvitationForm.relationType}>
              <option value="PARENT_OF">PARENT_OF</option>
              <option value="GUARDIAN_OF">GUARDIAN_OF</option>
            </select>
          </div>

          <div class="alert alert-info">
            <Info size={16} />
            <span>This will create a new internal user at /users/internal/[generated-id] with you as the {wardInvitationForm.relationType}.</span>
          </div>
        </div>
      {/if}

      {#if activeType === 'stewardship_request'}
        <div class="form-card">
          <h3><Shield size={20} /> Stewardship Request</h3>
          <p class="form-description">Request stewardship access to another user.</p>

          <div class="form-group">
            <label class="form-label" for="stew-ward">Ward Path *</label>
            <input
              type="text"
              id="stew-ward"
              class="form-input"
              bind:value={stewardshipRequestForm.wardPath}
              placeholder="/users/external/abc123"
            />
            <span class="form-hint">The user you want to become a steward of</span>
          </div>

          <div class="form-group">
            <label class="form-label" for="stew-type">Delegation Type</label>
            <select id="stew-type" class="form-input form-select" bind:value={stewardshipRequestForm.delegationType}>
              <option value="general">General</option>
              <option value="limited">Limited</option>
              <option value="emergency">Emergency</option>
            </select>
          </div>

          <div class="form-group">
            <label class="form-label" for="stew-scope">Scope (comma-separated)</label>
            <input
              type="text"
              id="stew-scope"
              class="form-input"
              bind:value={stewardshipRequestForm.scope}
              placeholder="read_profile,manage_tasks"
            />
          </div>

          <div class="form-group">
            <label class="form-label" for="stew-message">Message</label>
            <textarea
              id="stew-message"
              class="form-input form-textarea"
              bind:value={stewardshipRequestForm.message}
            ></textarea>
          </div>
        </div>
      {/if}

      <div class="form-actions">
        <button
          class="btn btn-primary btn-lg"
          onclick={sendMessage}
          disabled={sending}
        >
          {#if sending}
            Sending...
          {:else}
            <Send size={18} />
            Send Message
          {/if}
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .send-page {
    max-width: 900px;
    margin: 0 auto;
  }

  .header {
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

  .message-type-selector {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .message-type-selector h2 {
    font-size: 1rem;
    font-weight: 600;
    color: #374151;
    margin: 0 0 1rem 0;
  }

  .type-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 0.75rem;
  }

  .type-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.25rem;
    padding: 1rem;
    background: #f9fafb;
    border: 2px solid #e5e7eb;
    border-radius: 0.5rem;
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
  }

  .type-card:hover {
    border-color: #d1d5db;
    background: white;
  }

  .type-card.active {
    border-color: #6366f1;
    background: #eef2ff;
  }

  .type-card :global(svg) {
    color: #6366f1;
    margin-bottom: 0.25rem;
  }

  .type-label {
    font-weight: 600;
    color: #111827;
    font-size: 0.9375rem;
  }

  .type-desc {
    font-size: 0.75rem;
    color: #6b7280;
  }

  .form-container {
    margin-bottom: 2rem;
  }

  .form-card {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    margin-bottom: 1rem;
  }

  .form-card h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1.125rem;
    font-weight: 600;
    color: #111827;
    margin: 0 0 0.5rem 0;
  }

  .form-description {
    color: #6b7280;
    font-size: 0.875rem;
    margin: 0 0 1.5rem 0;
  }

  .form-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }

  .form-hint {
    display: block;
    font-size: 0.75rem;
    color: #9ca3af;
    margin-top: 0.25rem;
  }

  .radio-group {
    display: flex;
    gap: 1.5rem;
  }

  .radio-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    cursor: pointer;
    color: #374151;
  }

  .radio-label input[type="radio"] {
    width: 18px;
    height: 18px;
    accent-color: #6366f1;
  }

  .form-actions {
    display: flex;
    justify-content: flex-end;
  }

  .btn-lg {
    padding: 1rem 2rem;
    font-size: 1rem;
  }

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

  .result-path {
    display: block;
    margin-top: 0.5rem;
    font-family: monospace;
    font-size: 0.75rem;
    background: rgba(255, 255, 255, 0.5);
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
  }
</style>
