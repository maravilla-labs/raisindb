<script lang="ts">
  import type { ChatMessage as SDKChatMessage, PlanProjection } from '@raisindb/client';
  import { Check, Clock, Brain, Wrench, FileText, ListChecks, ChevronDown, ChevronRight } from 'lucide-svelte';
  import { marked } from 'marked';

  // Configure marked for safe rendering
  marked.setOptions({
    breaks: true,  // Convert \n to <br>
    gfm: true,     // GitHub Flavored Markdown
  });

  function renderMarkdown(text: string): string {
    if (!text) return '';
    try {
      return marked.parse(text) as string;
    } catch {
      return text;
    }
  }

  interface Props {
    message: SDKChatMessage;
    isMine: boolean;
    senderDisplayName?: string;
    /** Plan projections from ConversationStore snapshot (optional) */
    plans?: PlanProjection[];
    /** Callback to approve a pending plan */
    onApprovePlan?: (planPath: string) => Promise<void>;
    /** Callback to reject a pending plan */
    onRejectPlan?: (planPath: string, feedback?: string) => Promise<void>;
    /** Whether this is a streaming placeholder message */
    streaming?: boolean;
  }

  let {
    message,
    isMine,
    senderDisplayName = 'User',
    plans = [],
    onApprovePlan,
    onRejectPlan,
    streaming = false,
  }: Props = $props();

  let thoughtExpanded = $state(false);
  let toolResultExpanded = $state(false);
  let showToolDebug = $state(false);

  // Plan action state
  let planActionPending = $state(false);
  let planActionType = $state<'approve' | 'reject' | null>(null);
  let planActionError = $state<string | null>(null);
  let rejectingPlan = $state(false);
  let rejectionFeedback = $state('');

  if (typeof window !== 'undefined') {
    showToolDebug = window.localStorage.getItem('launchpad.chat.debugTools') === '1';
  }

  // Determine message type from SDK ChatMessage
  const messageType = $derived(message.messageType || 'chat');
  const data = $derived(message.data);

  // Check if this message is an AI intermediate type
  const isIntermediate = $derived(
    ['ai_thought', 'ai_tool_call', 'ai_tool_result', 'ai_plan', 'ai_task_update'].includes(messageType)
  );

  // Plan data from message itself (for ai_plan message types)
  const messagePlanData = $derived.by(() => {
    if (messageType === 'ai_plan' && data) return data;
    return null;
  });

  // Find matching plan projection from ConversationStore snapshot
  const matchingPlan = $derived.by((): PlanProjection | null => {
    if (!messagePlanData && messageType !== 'ai_plan') return null;
    const planPath = (messagePlanData?.plan_path || data?.plan_path || '') as string;
    const planId = (messagePlanData?.plan_id || data?.plan_id || '') as string;
    if (!planPath && !planId) return null;
    return plans.find(p =>
      (planPath && p.planPath === planPath) ||
      (planId && p.planId === planId)
    ) || null;
  });
  const isCanonicalPlanBubble = $derived.by(() => {
    if (messageType !== 'ai_plan') return true;
    if (!matchingPlan?.sourceMessagePath || !message.path) return true;
    return matchingPlan.sourceMessagePath === message.path;
  });

  // Derive plan UI state from projection (preferred) or raw message data
  const planTitle = $derived(matchingPlan?.title || (messagePlanData?.title as string) || (data?.title as string) || message.content || 'Plan');
  const planPath = $derived(matchingPlan?.planPath || (messagePlanData?.plan_path as string) || (data?.plan_path as string) || '');
  const planTasks = $derived(
    matchingPlan?.tasks ||
    (Array.isArray(messagePlanData?.tasks) ? (messagePlanData.tasks as any[]).map(t => ({
      title: typeof t === 'string' ? t : (t.title || ''),
      status: typeof t === 'string' ? 'pending' : (t.status || 'pending'),
    })) : [])
  );
  const planStatus = $derived(
    (planActionPending ? (planActionType === 'approve' ? 'in_progress' : 'cancelled') : null) ||
    matchingPlan?.status ||
    (messagePlanData?.status as string) ||
    (data?.status as string) ||
    ''
  );
  const isPlanAwaitingApproval = $derived(
    !!planPath && (
      planStatus === 'pending_approval' ||
      (!planStatus && (matchingPlan?.requiresApproval || messagePlanData?.requires_approval === true))
    )
  );
  const planStatusLabel = $derived.by(() => {
    switch (planStatus) {
      case 'pending_approval': return 'Pending Approval';
      case 'in_progress': return 'In Progress';
      case 'completed': return 'Completed';
      case 'cancelled': return 'Cancelled';
      default: return '';
    }
  });
  const planProcessingLabel = $derived.by(() => {
    if (planActionPending) {
      return planActionType === 'reject' ? 'Submitting rejection...' : 'Submitting approval...';
    }
    return '';
  });

  async function applyPlanAction(action: 'approve' | 'reject') {
    if (!planPath || planActionPending) return;
    planActionPending = true;
    planActionType = action;
    planActionError = null;
    try {
      if (action === 'approve') {
        await onApprovePlan?.(planPath);
      } else {
        await onRejectPlan?.(planPath, rejectionFeedback.trim() || undefined);
      }
      rejectingPlan = false;
      rejectionFeedback = '';
    } catch (e) {
      planActionError = e instanceof Error ? e.message : 'Plan action failed';
    } finally {
      planActionPending = false;
      planActionType = null;
    }
  }

  // Format timestamp to relative or time format
  function formatTime(dateStr: string | undefined): string {
    if (!dateStr) return '';
    const date = new Date(dateStr);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMins / 60);
    const diffDays = Math.floor(diffHours / 24);

    if (diffMins < 1) return 'now';
    if (diffMins < 60) return `${diffMins}m`;
    if (diffHours < 24) return `${diffHours}h`;
    if (diffDays < 7) return `${diffDays}d`;

    return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  }

  const content = $derived(message.content || '');
  const time = $derived(formatTime(message.timestamp));
  const senderName = $derived(senderDisplayName);
  const status = $derived(message.status || '');
</script>

{#if messageType === 'ai_thought'}
  <!-- Collapsible thinking section -->
  <div class="ai-intermediate thought">
    <button class="thought-toggle" onclick={() => thoughtExpanded = !thoughtExpanded}>
      <Brain size={14} />
      <span>Thinking...</span>
      {#if thoughtExpanded}
        <ChevronDown size={14} />
      {:else}
        <ChevronRight size={14} />
      {/if}
    </button>
    {#if thoughtExpanded}
      <div class="thought-content">{content}</div>
    {/if}
  </div>

{:else if messageType === 'ai_tool_call'}
  {#if showToolDebug}
    <!-- Tool call badge -->
    <div class="ai-intermediate tool-call">
      <div class="tool-badge">
        <Wrench size={14} />
        <span class="tool-name">{data?.tool_name || 'Tool'}</span>
        <span class="tool-status" class:running={data?.status === 'running'} class:completed={data?.status === 'completed'}>
          {data?.status || 'running'}
        </span>
      </div>
    </div>
  {/if}

{:else if messageType === 'ai_tool_result'}
  {#if showToolDebug}
    <!-- Tool result card -->
    <div class="ai-intermediate tool-result">
      <button class="result-toggle" onclick={() => toolResultExpanded = !toolResultExpanded}>
        <FileText size={14} />
        <span>{data?.tool_name || 'Result'}</span>
        {#if toolResultExpanded}
          <ChevronDown size={14} />
        {:else}
          <ChevronRight size={14} />
        {/if}
      </button>
      {#if toolResultExpanded}
        <div class="result-content">
          <pre>{typeof data?.result === 'string' ? data.result : JSON.stringify(data?.result, null, 2)}</pre>
        </div>
      {/if}
    </div>
  {/if}

{:else if messageType === 'ai_task_update'}
  <!-- Task updates are consumed by plan projection, not rendered as chat bubbles -->

{:else if messageType === 'ai_plan' && isCanonicalPlanBubble}
  <!-- Plan card, using ConversationStore plan projection when available -->
  <div class="ai-intermediate plan-card">
    <div class="plan-header">
      <ListChecks size={14} />
      <span>{planTitle}</span>
      {#if planStatusLabel}
        <span
          class="plan-status"
          class:pending={planStatus === 'pending_approval'}
          class:running={planStatus === 'in_progress'}
          class:done={planStatus === 'completed'}
          class:cancelled={planStatus === 'cancelled'}
        >
          {planStatusLabel}
        </span>
      {/if}
    </div>
    {#if planTasks.length > 0}
      <ul class="plan-tasks">
        {#each planTasks as task}
          <li class="plan-task" class:task-completed={task.status === 'completed'} class:task-in-progress={task.status === 'in_progress'}>
            {task.title}
            {#if task.status && task.status !== 'pending'}
              <span class="task-status-badge">{task.status}</span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if planProcessingLabel}
      <div class="plan-processing">{planProcessingLabel}</div>
    {/if}
    {#if isPlanAwaitingApproval && onApprovePlan && onRejectPlan}
      <div class="plan-actions">
        {#if !rejectingPlan}
          <button class="plan-btn approve" disabled={planActionPending} onclick={() => applyPlanAction('approve')}>Approve</button>
          <button class="plan-btn reject" disabled={planActionPending} onclick={() => { rejectingPlan = true; }}>Reject</button>
        {:else}
          <textarea
            class="reject-feedback"
            bind:value={rejectionFeedback}
            rows="2"
            placeholder="Optional feedback for revision"
          ></textarea>
          <div class="plan-actions-inline">
            <button class="plan-btn reject" disabled={planActionPending} onclick={() => applyPlanAction('reject')}>Confirm Reject</button>
            <button class="plan-btn neutral" disabled={planActionPending} onclick={() => { rejectingPlan = false; rejectionFeedback = ''; }}>Cancel</button>
          </div>
        {/if}
        {#if planActionError}
          <div class="plan-error">{planActionError}</div>
        {/if}
      </div>
    {/if}
  </div>

{:else}
  <!-- Regular chat message -->
  <div class="chat-message" class:mine={isMine} class:streaming>
    <div class="sender-name">
      {isMine ? 'Me' : senderName}
      {#if data?.model && !isMine}
        <span class="model-badge">{data.model}</span>
      {/if}
    </div>
    <div class="bubble" class:markdown-content={!isMine}>
      {#if isMine}
        {content}
      {:else}
        {@html renderMarkdown(content)}
      {/if}
      {#if streaming}
        <span class="streaming-cursor"></span>
      {/if}
    </div>
    <div class="timestamp">
      {time}
      {#if isMine}
        <span class="status-icon" class:pending={status === 'pending'} class:sent={status === 'sent'} class:delivered={status === 'delivered'} class:read={status === 'read'}>
          {#if status === 'pending'}
            <Clock size={12} />
          {:else if status === 'sent'}
            <Check size={12} />
          {:else if status === 'delivered' || status === 'read'}
            <span class="double-check">
              <Check size={12} />
              <Check size={12} />
            </span>
          {/if}
        </span>
      {/if}
    </div>

    <!-- Inline plan cards from message children (for rich messages loaded from node tree) -->
    {#if !isMine && Array.isArray(message.children)}
      {#each message.children as child (child.id)}
        {#if child.type === 'thought'}
          <div class="inline-thought">
            <button class="thought-toggle" onclick={() => thoughtExpanded = !thoughtExpanded}>
              <Brain size={14} />
              <span>Thinking...</span>
              {#if thoughtExpanded}
                <ChevronDown size={14} />
              {:else}
                <ChevronRight size={14} />
              {/if}
            </button>
            {#if thoughtExpanded}
              <div class="thought-content">{child.content}</div>
            {/if}
          </div>
        {:else if child.type === 'plan'}
          <div class="inline-plan plan-card">
            <div class="plan-header">
              <ListChecks size={14} />
              <span>{child.planTitle || 'Plan'}</span>
              {#if child.status}
                <span
                  class="plan-status"
                  class:pending={child.status === 'pending_approval'}
                  class:running={child.status === 'in_progress'}
                  class:done={child.status === 'completed'}
                  class:cancelled={child.status === 'cancelled'}
                >
                  {child.status}
                </span>
              {/if}
            </div>
            {#if child.tasks && child.tasks.length > 0}
              <ul class="plan-tasks">
                {#each child.tasks as task}
                  <li>{task.title}</li>
                {/each}
              </ul>
            {/if}
          </div>
        {:else if child.type === 'tool_call' && showToolDebug}
          <div class="inline-tool">
            <div class="tool-badge">
              <Wrench size={14} />
              <span class="tool-name">{child.toolName || 'Tool'}</span>
              <span class="tool-status" class:running={child.status === 'running'} class:completed={child.status === 'completed'}>
                {child.status || 'completed'}
              </span>
            </div>
          </div>
        {/if}
      {/each}
    {/if}
  </div>
{/if}

<style>
  /* --- Regular chat message styles --- */
  .chat-message {
    display: flex;
    flex-direction: column;
    max-width: 75%;
    margin-bottom: 0.75rem;
    animation: slideIn 0.3s ease-out;
  }

  @keyframes slideIn {
    from { opacity: 0; transform: translateY(10px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .chat-message.mine {
    align-self: flex-end;
    align-items: flex-end;
    animation-name: slideInRight;
  }

  @keyframes slideInRight {
    from { opacity: 0; transform: translateX(10px); }
    to { opacity: 1; transform: translateX(0); }
  }

  .chat-message:not(.mine) {
    align-self: flex-start;
    align-items: flex-start;
    animation-name: slideInLeft;
  }

  @keyframes slideInLeft {
    from { opacity: 0; transform: translateX(-10px); }
    to { opacity: 1; transform: translateX(0); }
  }

  .sender-name {
    font-size: 0.75rem;
    color: var(--color-text-muted);
    margin-bottom: 0.25rem;
    padding: 0 0.5rem;
    display: flex;
    align-items: center;
    gap: 0.375rem;
  }

  .chat-message.mine .sender-name {
    text-align: right;
  }

  .model-badge {
    font-size: 0.625rem;
    background: #eef2ff;
    color: #6366f1;
    padding: 0.0625rem 0.375rem;
    border-radius: 4px;
    font-weight: 500;
  }

  .bubble {
    padding: 0.625rem 1rem;
    border-radius: 1.25rem;
    font-size: 0.9375rem;
    line-height: 1.4;
    word-wrap: break-word;
    white-space: pre-wrap;
  }

  .bubble.markdown-content {
    white-space: normal;
  }

  .bubble.markdown-content :global(p) {
    margin: 0 0 0.5rem 0;
  }

  .bubble.markdown-content :global(p:last-child) {
    margin-bottom: 0;
  }

  .bubble.markdown-content :global(ul) {
    margin: 0.25rem 0;
    padding-left: 1.25rem;
    list-style-type: disc;
  }

  .bubble.markdown-content :global(ol) {
    margin: 0.25rem 0;
    padding-left: 1.25rem;
    list-style-type: decimal;
  }

  .bubble.markdown-content :global(li) {
    margin: 0.125rem 0;
    display: list-item;
  }

  .bubble.markdown-content :global(strong) {
    font-weight: 600;
  }

  .bubble.markdown-content :global(em) {
    font-style: italic;
  }

  .bubble.markdown-content :global(code) {
    background: rgba(0, 0, 0, 0.06);
    padding: 0.125rem 0.25rem;
    border-radius: 3px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 0.8125rem;
  }

  .bubble.markdown-content :global(pre) {
    background: var(--color-bg);
    color: var(--color-text-secondary);
    padding: 0.5rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--color-border);
    overflow-x: auto;
    margin: 0.5rem 0;
  }

  .bubble.markdown-content :global(pre code) {
    background: transparent;
    padding: 0;
  }

  .bubble.markdown-content :global(blockquote) {
    border-left: 3px solid var(--color-accent);
    margin: 0.5rem 0;
    padding-left: 0.75rem;
    color: var(--color-text-secondary);
  }

  .bubble.markdown-content :global(a) {
    color: var(--color-accent);
    text-decoration: underline;
  }

  .bubble.markdown-content :global(h1),
  .bubble.markdown-content :global(h2),
  .bubble.markdown-content :global(h3) {
    margin: 0.5rem 0 0.25rem 0;
    font-weight: 600;
    color: var(--color-text-heading);
  }

  .bubble.markdown-content :global(h1) { font-size: 1.125rem; }
  .bubble.markdown-content :global(h2) { font-size: 1rem; }
  .bubble.markdown-content :global(h3) { font-size: 0.9375rem; }

  .chat-message.mine .bubble {
    background: linear-gradient(135deg, var(--color-accent), #b8962e);
    color: var(--color-bg);
    border-bottom-right-radius: 0.25rem;
  }

  .chat-message:not(.mine) .bubble {
    background: var(--color-surface);
    color: var(--color-text);
    border-bottom-left-radius: 0.25rem;
  }

  .streaming-cursor {
    display: inline-block;
    width: 2px;
    height: 1em;
    background: var(--color-accent);
    margin-left: 2px;
    animation: blink 1s step-end infinite;
    vertical-align: text-bottom;
  }

  @keyframes blink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0; }
  }

  .timestamp {
    font-size: 0.6875rem;
    color: var(--color-text-muted);
    margin-top: 0.25rem;
    padding: 0 0.5rem;
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .status-icon {
    display: inline-flex;
    align-items: center;
    transition: color 0.3s ease;
  }

  .status-icon.pending {
    color: var(--color-text-muted);
  }

  .status-icon.sent {
    color: var(--color-text-muted);
  }

  .status-icon.delivered {
    color: var(--color-accent);
  }

  .status-icon.read {
    color: var(--color-success);
  }

  .double-check {
    display: inline-flex;
    align-items: center;
  }

  .double-check :global(svg:last-child) {
    margin-left: -6px;
  }

  /* --- AI intermediate message styles --- */
  .ai-intermediate {
    align-self: flex-start;
    max-width: 85%;
    margin-bottom: 0.5rem;
    animation: slideIn 0.3s ease-out;
  }

  /* Inline children styles */
  .inline-thought, .inline-plan, .inline-tool {
    margin-top: 0.375rem;
  }

  /* Thought */
  .thought-toggle {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.75rem;
    background: #faf5ff;
    border: 1px solid #e9d5ff;
    border-radius: 8px;
    color: #7c3aed;
    font-size: 0.8125rem;
    cursor: pointer;
    transition: background 0.15s;
  }

  .thought-toggle:hover {
    background: #f3e8ff;
  }

  .thought-content {
    margin-top: 0.375rem;
    padding: 0.625rem 0.75rem;
    background: #faf5ff;
    border: 1px solid #e9d5ff;
    border-radius: 8px;
    color: #6b21a8;
    font-size: 0.8125rem;
    line-height: 1.5;
    white-space: pre-wrap;
  }

  /* Tool call */
  .tool-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.75rem;
    background: #fffbeb;
    border: 1px solid #fde68a;
    border-radius: 8px;
    font-size: 0.8125rem;
    color: #92400e;
  }

  .tool-name {
    font-weight: 500;
    font-family: 'SF Mono', 'Fira Code', monospace;
  }

  .tool-status {
    font-size: 0.6875rem;
    padding: 0.0625rem 0.375rem;
    border-radius: 4px;
    font-weight: 500;
  }

  .tool-status.running {
    background: #fef3c7;
    color: #d97706;
  }

  .tool-status.completed {
    background: #d1fae5;
    color: #059669;
  }

  /* Tool result */
  .result-toggle {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.75rem;
    background: #f0fdf4;
    border: 1px solid #bbf7d0;
    border-radius: 8px;
    color: #166534;
    font-size: 0.8125rem;
    cursor: pointer;
    transition: background 0.15s;
  }

  .result-toggle:hover {
    background: #dcfce7;
  }

  .result-content {
    margin-top: 0.375rem;
    padding: 0.625rem 0.75rem;
    background: #f0fdf4;
    border: 1px solid #bbf7d0;
    border-radius: 8px;
    overflow-x: auto;
  }

  .result-content pre {
    margin: 0;
    font-size: 0.75rem;
    color: #166534;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: 'SF Mono', 'Fira Code', monospace;
  }

  /* Plan card */
  .plan-card {
    padding: 0.75rem;
    background: #eff6ff;
    border: 1px solid #bfdbfe;
    border-radius: 8px;
  }

  .plan-header {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    color: #1e40af;
    font-size: 0.875rem;
    font-weight: 500;
    margin-bottom: 0.5rem;
  }

  .plan-status {
    margin-left: auto;
    font-size: 0.6875rem;
    font-weight: 700;
    padding: 0.15rem 0.45rem;
    border-radius: 999px;
    letter-spacing: 0.01em;
    border: 1px solid transparent;
  }

  .plan-status.pending {
    background: #fef3c7;
    color: #92400e;
    border-color: #f59e0b;
  }

  .plan-status.running {
    background: #dbeafe;
    color: #1d4ed8;
    border-color: #60a5fa;
  }

  .plan-status.done {
    background: #dcfce7;
    color: #166534;
    border-color: #4ade80;
  }

  .plan-status.cancelled {
    background: #fee2e2;
    color: #991b1b;
    border-color: #f87171;
  }

  .plan-tasks {
    margin: 0;
    padding-left: 1.5rem;
    list-style: disc;
  }

  .plan-tasks li {
    font-size: 0.8125rem;
    color: #1e40af;
    line-height: 1.6;
  }

  .plan-task.task-completed {
    text-decoration: line-through;
    opacity: 0.7;
  }

  .plan-task.task-in-progress {
    font-weight: 600;
  }

  .task-status-badge {
    font-size: 0.6rem;
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
    background: #dbeafe;
    color: #1d4ed8;
    margin-left: 0.375rem;
    font-weight: 500;
  }

  .plan-actions {
    margin-top: 0.625rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .plan-processing {
    margin-top: 0.4rem;
    font-size: 0.75rem;
    color: #1d4ed8;
    font-weight: 600;
  }

  .plan-actions-inline {
    display: flex;
    gap: 0.5rem;
  }

  .plan-btn {
    border: none;
    border-radius: 6px;
    padding: 0.35rem 0.65rem;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
  }

  .plan-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .plan-btn.approve {
    background: #16a34a;
    color: #fff;
  }

  .plan-btn.reject {
    background: #dc2626;
    color: #fff;
  }

  .plan-btn.neutral {
    background: #e5e7eb;
    color: #111827;
  }

  .reject-feedback {
    width: 100%;
    border: 1px solid #cbd5e1;
    border-radius: 6px;
    padding: 0.4rem 0.5rem;
    font-size: 0.75rem;
    resize: vertical;
    font-family: inherit;
  }

  .plan-error {
    color: #b91c1c;
    font-size: 0.75rem;
    font-weight: 500;
  }
</style>
