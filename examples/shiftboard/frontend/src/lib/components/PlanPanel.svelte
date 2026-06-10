<script lang="ts">
  /**
   * The plan panel: renders the SDK's deterministic plan projection
   * (snapshot.plans) as proposal/progress cards.
   *
   * - pending_approval + requiresApproval → Approve / Reject buttons with an
   *   optional feedback line (store.approvePlan / store.rejectPlan)
   * - executing → live task status icons (pending/in_progress/completed)
   * - step_by_step pause → the agent's last message has finish_reason
   *   awaiting_step_continue → Continue button (a normal 'continue' message
   *   resumes the run)
   */
  import type { AgentChatState } from '../stores/chat.svelte';

  let { chat }: { chat: AgentChatState } = $props();

  let busyPlan = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let feedback = $state('');

  const STATUS_LABELS: Record<string, string> = {
    pending_approval: 'Awaiting approval',
    approved: 'Approved',
    executing: 'Executing',
    in_progress: 'Executing',
    completed: 'Completed',
    failed: 'Failed',
    cancelled: 'Cancelled',
  };

  // step_by_step pause: the turn ends with finish_reason
  // awaiting_step_continue. Resume with a normal 'continue' message.
  // (Skip trailing delivery-mirror messages that carry no finishReason.)
  const isPaused = $derived.by(() => {
    if (chat.snapshot.isStreaming) return false;
    const last = [...chat.snapshot.messages]
      .reverse()
      .find((m) => m.role === 'assistant' && m.finishReason);
    return last?.finishReason === 'awaiting_step_continue';
  });

  async function act(planPath: string, action: 'approve' | 'reject') {
    actionError = null;
    busyPlan = planPath;
    try {
      if (action === 'approve') {
        await chat.approvePlan(planPath);
      } else {
        await chat.rejectPlan(
          planPath,
          feedback.trim() || 'Rejected from the Shiftboard plan panel.',
        );
      }
      feedback = '';
    } catch (err) {
      actionError = err instanceof Error ? err.message : String(err);
    } finally {
      busyPlan = null;
    }
  }

  function sendContinue() {
    chat.send('continue').catch((err) => {
      actionError = err instanceof Error ? err.message : String(err);
    });
  }
</script>

<section class="panel plan-panel" data-testid="plan-panel">
  <h2 class="panel-title">Plans</h2>

  <div class="plan-scroll">
    {#if chat.snapshot.plans.length === 0}
      <p class="muted">
        No plans yet. Ask the coordinator to “Fill all open weekend shifts”
        and the proposal shows up here for approval. Approving it starts one
        durable fill-shift workflow per open shift — a completed task means
        the workflow was <em>started</em>; the board fills as staff accept
        their inbox tasks.
      </p>
    {/if}

    {#if actionError}
      <p class="error-banner" role="alert">{actionError}</p>
    {/if}

    {#each chat.snapshot.plans as plan (plan.planPath ?? plan.key)}
      {@const statusClass = plan.status.replace(/[^a-z_]/gi, '')}
      <article
        class="plan-card {statusClass}"
        data-testid="plan-card"
        data-status={plan.status}
      >
        <div class="plan-head">
          <h3 class="plan-title">{plan.title}</h3>
          <span class="plan-status {statusClass}" data-testid="plan-status">
            {STATUS_LABELS[plan.status] ?? plan.status}
          </span>
        </div>

        <ol class="plan-task-list">
          {#each plan.tasks as task, i (task.taskId ?? `${i}-${task.title}`)}
            <li class="task-row {task.status}" data-testid="task-row" data-status={task.status}>
              <span class="task-icon {task.status}">
                {#if task.status === 'completed'}✓
                {:else if task.status === 'failed'}✗
                {:else if task.status === 'cancelled'}–
                {/if}
              </span>
              <span class="task-row-title">{task.title}</span>
            </li>
          {/each}
        </ol>

        {#if plan.requiresApproval && plan.planPath}
          <input
            class="plan-feedback"
            type="text"
            placeholder="Optional feedback for a rejection…"
            data-testid="plan-feedback"
            bind:value={feedback}
          />
          <div class="plan-actions">
            <button
              class="approve"
              data-testid="plan-approve"
              disabled={busyPlan === plan.planPath}
              onclick={() => act(plan.planPath!, 'approve')}
            >
              Approve
            </button>
            <button
              class="reject"
              data-testid="plan-reject"
              disabled={busyPlan === plan.planPath}
              onclick={() => act(plan.planPath!, 'reject')}
            >
              Reject
            </button>
          </div>
        {/if}
      </article>
    {/each}

    {#if isPaused}
      <div class="continue-bar" data-testid="continue-bar">
        <p>Step done — the agent paused (step-by-step mode).</p>
        <button data-testid="plan-continue" onclick={sendContinue}>Continue ▸</button>
      </div>
    {/if}
  </div>
</section>
