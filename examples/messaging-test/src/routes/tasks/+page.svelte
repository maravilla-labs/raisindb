<script lang="ts">
  import { onMount } from 'svelte';
  import { user } from '$lib/stores/auth';
  import { messagesStore } from '$lib/stores/messages';
  import { query, ACCESS_CONTROL_WORKSPACE, type MessageNode } from '$lib/raisin';
  import {
    ClipboardList, CheckCircle, XCircle, RefreshCw, AlertCircle,
    Clock, Calendar, User, Send, Filter, CheckSquare
  } from 'lucide-svelte';

  interface TaskNode {
    id: string;
    path: string;
    name: string;
    node_type: string;
    properties: {
      task_type?: string;
      title?: string;
      description?: string;
      priority?: number;
      status?: string;
      options?: string[];
      due_date?: string;
      assignee_id?: string;
      assigner_id?: string;
      created_at?: string;
      [key: string]: unknown;
    };
  }

  type Tab = 'my-tasks' | 'assigned';
  type StatusFilter = 'all' | 'pending' | 'completed';

  let activeTab = $state<Tab>('my-tasks');
  let statusFilter = $state<StatusFilter>('all');
  let loading = $state(true);
  let actionLoading = $state<string | null>(null);
  let actionResult = $state<{ success: boolean; message: string } | null>(null);

  let myTasks = $state<TaskNode[]>([]);
  let assignedTasks = $state<MessageNode[]>([]);

  function getUserHomePath(): string {
    if (!$user?.home) return '';
    return $user.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');
  }

  async function loadMyTasks() {
    if (!$user?.home) return;

    const homePath = getUserHomePath();

    try {
      const tasks = await query<TaskNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE CHILD_OF('${homePath}/inbox')
          AND node_type = 'raisin:InboxTask'
        ORDER BY properties->>'priority' ASC, properties->>'created_at' DESC
        LIMIT 100
      `);
      myTasks = tasks;
    } catch (err) {
      console.error('[tasks] Failed to load my tasks:', err);
      myTasks = [];
    }
  }

  async function loadAssignedTasks() {
    if (!$user?.home) return;

    const homePath = getUserHomePath();

    try {
      const tasks = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('${homePath}/sent')
          AND node_type = 'raisin:Message'
          AND properties->>'message_type' = 'task_assignment'
        ORDER BY properties->>'created_at' DESC
        LIMIT 100
      `);
      assignedTasks = tasks;
    } catch (err) {
      console.error('[tasks] Failed to load assigned tasks:', err);
      assignedTasks = [];
    }
  }

  async function refresh() {
    loading = true;
    try {
      await Promise.all([loadMyTasks(), loadAssignedTasks()]);
    } finally {
      loading = false;
    }
  }

  async function completeTask(task: TaskNode, response?: string) {
    actionLoading = task.id;
    actionResult = null;

    try {
      // Update the task status to completed
      await query(`
        UPDATE '${ACCESS_CONTROL_WORKSPACE}'
        SET properties = properties || '{"status": "completed", "response": ${JSON.stringify(response || 'completed')}}'::jsonb
        WHERE path = $1
      `, [task.path]);

      actionResult = { success: true, message: 'Task completed!' };
      await refresh();
    } catch (err) {
      console.error('[tasks] Failed to complete task:', err);
      actionResult = { success: false, message: err instanceof Error ? err.message : 'Failed to complete task' };
    } finally {
      actionLoading = null;
    }
  }

  // Filtered tasks
  let filteredMyTasks = $derived(
    myTasks.filter(t => {
      if (statusFilter === 'all') return true;
      if (statusFilter === 'pending') return t.properties.status !== 'completed';
      if (statusFilter === 'completed') return t.properties.status === 'completed';
      return true;
    })
  );

  function getPriorityLabel(priority?: number): { label: string; class: string } {
    switch (priority) {
      case 1: return { label: 'Low', class: 'priority-low' };
      case 2: return { label: 'Low', class: 'priority-low' };
      case 3: return { label: 'Normal', class: 'priority-normal' };
      case 4: return { label: 'High', class: 'priority-high' };
      case 5: return { label: 'Urgent', class: 'priority-urgent' };
      default: return { label: 'Normal', class: 'priority-normal' };
    }
  }

  function getTaskTypeIcon(taskType?: string) {
    switch (taskType) {
      case 'approval': return CheckSquare;
      case 'input': return ClipboardList;
      case 'review': return ClipboardList;
      case 'action': return CheckCircle;
      default: return ClipboardList;
    }
  }

  function formatDate(dateStr: string | undefined): string {
    if (!dateStr) return 'No due date';
    try {
      return new Date(dateStr).toLocaleDateString();
    } catch {
      return dateStr;
    }
  }

  function formatDateTime(dateStr: string | undefined): string {
    if (!dateStr) return 'Unknown';
    try {
      return new Date(dateStr).toLocaleString();
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

<div class="tasks-page">
  <div class="header">
    <div class="title-section">
      <h1>
        <ClipboardList size={28} />
        Tasks
      </h1>
      <p>View and manage your assigned tasks</p>
    </div>
    <button class="btn btn-secondary" onclick={refresh} disabled={loading}>
      <span class:spinning={loading}><RefreshCw size={18} /></span>
      Refresh
    </button>
  </div>

  {#if !$user}
    <div class="alert alert-info">
      <AlertCircle size={18} />
      <span>Login to view your tasks.</span>
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
          class:active={activeTab === 'my-tasks'}
          onclick={() => activeTab = 'my-tasks'}
        >
          <ClipboardList size={18} />
          My Tasks ({myTasks.length})
        </button>
        <button
          class="tab"
          class:active={activeTab === 'assigned'}
          onclick={() => activeTab = 'assigned'}
        >
          <Send size={18} />
          Assigned by Me ({assignedTasks.length})
        </button>
      </div>
    </div>

    {#if loading}
      <div class="loading">
        <span class="spinning"><RefreshCw size={24} /></span>
        <span>Loading tasks...</span>
      </div>
    {:else}
      {#if activeTab === 'my-tasks'}
        <div class="content-card">
          <div class="filter-bar">
            <Filter size={16} />
            <span>Filter:</span>
            <div class="filter-buttons">
              <button
                class="filter-btn"
                class:active={statusFilter === 'all'}
                onclick={() => statusFilter = 'all'}
              >
                All
              </button>
              <button
                class="filter-btn"
                class:active={statusFilter === 'pending'}
                onclick={() => statusFilter = 'pending'}
              >
                Pending
              </button>
              <button
                class="filter-btn"
                class:active={statusFilter === 'completed'}
                onclick={() => statusFilter = 'completed'}
              >
                Completed
              </button>
            </div>
          </div>

          {#if filteredMyTasks.length === 0}
            <div class="empty-state-inline">
              <ClipboardList size={32} />
              <p>{statusFilter === 'all' ? 'No tasks assigned to you' : `No ${statusFilter} tasks`}</p>
            </div>
          {:else}
            <div class="tasks-list">
              {#each filteredMyTasks as task (task.id)}
                {@const TaskIcon = getTaskTypeIcon(task.properties.task_type)}
                {@const priority = getPriorityLabel(task.properties.priority)}
                {@const isCompleted = task.properties.status === 'completed'}
                <div class="task-card" class:completed={isCompleted}>
                  <div class="task-header">
                    <div class="task-type">
                      <svelte:component this={TaskIcon} size={16} />
                      <span>{task.properties.task_type || 'Task'}</span>
                    </div>
                    <div class="task-badges">
                      <span class="badge {priority.class}">{priority.label}</span>
                      <span class="badge" class:badge-success={isCompleted} class:badge-warning={!isCompleted}>
                        {isCompleted ? 'Completed' : 'Pending'}
                      </span>
                    </div>
                  </div>

                  <div class="task-title">{task.properties.title || task.name}</div>

                  {#if task.properties.description}
                    <div class="task-description">{task.properties.description}</div>
                  {/if}

                  <div class="task-meta">
                    {#if task.properties.assigner_id}
                      <div class="meta-item">
                        <User size={14} />
                        <span>From: {task.properties.assigner_id}</span>
                      </div>
                    {/if}
                    {#if task.properties.due_date}
                      <div class="meta-item">
                        <Calendar size={14} />
                        <span>Due: {formatDate(task.properties.due_date)}</span>
                      </div>
                    {/if}
                    <div class="meta-item">
                      <Clock size={14} />
                      <span>{formatDateTime(task.properties.created_at)}</span>
                    </div>
                  </div>

                  {#if !isCompleted}
                    <div class="task-actions">
                      {#if task.properties.task_type === 'approval' && task.properties.options}
                        {#each task.properties.options as option}
                          <button
                            class="btn btn-sm"
                            class:btn-success={option.toLowerCase() === 'approve' || option.toLowerCase() === 'accept'}
                            class:btn-danger={option.toLowerCase() === 'reject' || option.toLowerCase() === 'decline'}
                            class:btn-secondary={option.toLowerCase() !== 'approve' && option.toLowerCase() !== 'accept' && option.toLowerCase() !== 'reject' && option.toLowerCase() !== 'decline'}
                            onclick={() => completeTask(task, option)}
                            disabled={actionLoading === task.id}
                          >
                            {option}
                          </button>
                        {/each}
                      {:else if task.properties.task_type === 'review'}
                        <button
                          class="btn btn-sm btn-success"
                          onclick={() => completeTask(task, 'reviewed')}
                          disabled={actionLoading === task.id}
                        >
                          <CheckCircle size={16} />
                          Mark as Reviewed
                        </button>
                      {:else}
                        <button
                          class="btn btn-sm btn-success"
                          onclick={() => completeTask(task, 'completed')}
                          disabled={actionLoading === task.id}
                        >
                          <CheckCircle size={16} />
                          Mark Complete
                        </button>
                      {/if}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      {#if activeTab === 'assigned'}
        <div class="content-card">
          {#if assignedTasks.length === 0}
            <div class="empty-state-inline">
              <Send size={32} />
              <p>You haven't assigned any tasks yet</p>
              <a href="/send" class="btn btn-primary btn-sm">
                Assign a Task
              </a>
            </div>
          {:else}
            <div class="tasks-list">
              {#each assignedTasks as task (task.id)}
                {@const body = task.properties.body as Record<string, unknown> | undefined}
                {@const isCompleted = task.properties.status === 'completed' || task.properties.status === 'delivered'}
                <div class="task-card assigned-task">
                  <div class="task-header">
                    <div class="task-type">
                      <ClipboardList size={16} />
                      <span>{body?.task_type || 'Task'}</span>
                    </div>
                    <span class="badge" class:badge-success={task.properties.status === 'delivered'} class:badge-warning={task.properties.status === 'pending'} class:badge-info={task.properties.status === 'sent'}>
                      {task.properties.status}
                    </span>
                  </div>

                  <div class="task-title">{body?.title || task.properties.subject || task.name}</div>

                  {#if body?.description}
                    <div class="task-description">{body.description}</div>
                  {/if}

                  <div class="task-meta">
                    {#if body?.assignee_id}
                      <div class="meta-item">
                        <User size={14} />
                        <span>Assigned to: {body.assignee_id}</span>
                      </div>
                    {/if}
                    <div class="meta-item">
                      <Clock size={14} />
                      <span>{formatDateTime(task.properties.created_at)}</span>
                    </div>
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
  .tasks-page {
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

  .filter-bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1.5rem;
    padding-bottom: 1rem;
    border-bottom: 1px solid #e5e7eb;
    color: #6b7280;
    font-size: 0.875rem;
  }

  .filter-buttons {
    display: flex;
    gap: 0.5rem;
  }

  .filter-btn {
    padding: 0.375rem 0.75rem;
    border: 1px solid #e5e7eb;
    background: white;
    border-radius: 0.375rem;
    font-size: 0.8125rem;
    color: #6b7280;
    cursor: pointer;
    transition: all 0.2s;
  }

  .filter-btn:hover {
    background: #f9fafb;
    border-color: #d1d5db;
  }

  .filter-btn.active {
    background: #eef2ff;
    border-color: #6366f1;
    color: #6366f1;
  }

  .empty-state-inline {
    text-align: center;
    padding: 3rem;
    color: #9ca3af;
  }

  .empty-state-inline p {
    margin: 0.5rem 0 1rem 0;
  }

  .tasks-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .task-card {
    padding: 1.25rem;
    background: #f9fafb;
    border-radius: 0.5rem;
    border: 1px solid #e5e7eb;
  }

  .task-card.completed {
    opacity: 0.7;
    background: #f3f4f6;
  }

  .task-card.assigned-task {
    background: #fefce8;
    border-color: #fef08a;
  }

  .task-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.75rem;
  }

  .task-type {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: #6b7280;
    text-transform: uppercase;
  }

  .task-badges {
    display: flex;
    gap: 0.5rem;
  }

  .task-title {
    font-size: 1.125rem;
    font-weight: 600;
    color: #111827;
    margin-bottom: 0.5rem;
  }

  .task-description {
    font-size: 0.9375rem;
    color: #4b5563;
    margin-bottom: 1rem;
    line-height: 1.5;
  }

  .task-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    font-size: 0.8125rem;
    color: #6b7280;
    margin-bottom: 1rem;
  }

  .meta-item {
    display: flex;
    align-items: center;
    gap: 0.375rem;
  }

  .task-actions {
    display: flex;
    gap: 0.5rem;
    padding-top: 0.75rem;
    border-top: 1px solid #e5e7eb;
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

  .badge-warning {
    background: #fef3c7;
    color: #92400e;
  }

  .badge-info {
    background: #dbeafe;
    color: #1e40af;
  }

  .priority-low {
    background: #f3f4f6;
    color: #6b7280;
  }

  .priority-normal {
    background: #dbeafe;
    color: #1e40af;
  }

  .priority-high {
    background: #fef3c7;
    color: #92400e;
  }

  .priority-urgent {
    background: #fecaca;
    color: #991b1b;
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
