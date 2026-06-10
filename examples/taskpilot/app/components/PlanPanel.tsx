/**
 * The plan panel: renders the SDK's deterministic plan projection
 * (chat.plans) as proposal/progress cards.
 *
 * - pending_approval + requiresApproval → Approve / Reject buttons
 *   (store.approvePlan / store.rejectPlan)
 * - executing → live task status icons (pending/in_progress/completed)
 * - step_by_step pause → the agent's last message has finish_reason
 *   awaiting_step_continue → Continue button (a normal 'continue' message
 *   resumes the run)
 */
import { useState } from 'react';
import type { UseConversationReturn } from '@raisindb/client/react';

const STATUS_LABELS: Record<string, string> = {
  pending_approval: 'Awaiting approval',
  approved: 'Approved',
  executing: 'Executing',
  in_progress: 'Executing',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

function TaskIcon({ status }: { status: string }) {
  if (status === 'completed') return <span className="task-icon completed">✓</span>;
  if (status === 'in_progress') return <span className="task-icon in-progress" />;
  if (status === 'failed') return <span className="task-icon failed">✗</span>;
  if (status === 'cancelled') return <span className="task-icon cancelled">–</span>;
  return <span className="task-icon pending" />;
}

export function PlanPanel({ chat }: { chat: UseConversationReturn }) {
  const [busyPlan, setBusyPlan] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  // step_by_step pause: the turn ends with finish_reason
  // awaiting_step_continue. Resume with a normal 'continue' message.
  // (Skip trailing delivery-mirror messages that carry no finishReason.)
  const lastWithReason = [...chat.messages]
    .reverse()
    .find((m) => m.role === 'assistant' && m.finishReason);
  const isPaused =
    !chat.isStreaming && lastWithReason?.finishReason === 'awaiting_step_continue';

  async function approve(planPath: string) {
    setActionError(null);
    setBusyPlan(planPath);
    try {
      await chat.approvePlan(planPath);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyPlan(null);
    }
  }

  async function reject(planPath: string) {
    setActionError(null);
    setBusyPlan(planPath);
    try {
      await chat.rejectPlan(planPath, 'Rejected from the TaskPilot plan panel.');
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyPlan(null);
    }
  }

  return (
    <section className="panel plan-panel" data-testid="plan-panel">
      <h2 className="panel-title">Plans</h2>

      {chat.plans.length === 0 && (
        <p className="muted">
          No plans yet. Ask Pilot to “Plan and execute” something and the
          proposal shows up here.
        </p>
      )}

      {actionError && (
        <p className="error-banner" role="alert">
          {actionError}
        </p>
      )}

      {chat.plans.map((plan) => {
        const key = plan.planPath ?? plan.key;
        const statusClass = plan.status.replace(/[^a-z_]/gi, '');
        return (
          <article
            className={`plan-card ${statusClass}`}
            key={key}
            data-testid="plan-card"
            data-status={plan.status}
          >
            <div className="plan-head">
              <h3 className="plan-title">{plan.title}</h3>
              <span className={`plan-status ${statusClass}`} data-testid="plan-status">
                {STATUS_LABELS[plan.status] ?? plan.status}
              </span>
            </div>

            <ol className="task-list">
              {plan.tasks.map((task, i) => (
                <li
                  key={task.taskId ?? `${i}-${task.title}`}
                  className={`task-row ${task.status}`}
                  data-testid="task-row"
                  data-status={task.status}
                >
                  <TaskIcon status={task.status} />
                  <span className="task-title">{task.title}</span>
                </li>
              ))}
            </ol>

            {plan.requiresApproval && plan.planPath && (
              <div className="plan-actions">
                <button
                  className="approve"
                  data-testid="plan-approve"
                  disabled={busyPlan === plan.planPath}
                  onClick={() => approve(plan.planPath!)}
                >
                  Approve
                </button>
                <button
                  className="reject"
                  data-testid="plan-reject"
                  disabled={busyPlan === plan.planPath}
                  onClick={() => reject(plan.planPath!)}
                >
                  Reject
                </button>
              </div>
            )}
          </article>
        );
      })}

      {isPaused && (
        <div className="continue-bar" data-testid="continue-bar">
          <p>Step done — the agent paused (step-by-step mode).</p>
          <button
            data-testid="plan-continue"
            onClick={() => {
              chat.sendMessage('continue').catch((err) => {
                setActionError(err instanceof Error ? err.message : String(err));
              });
            }}
          >
            Continue ▸
          </button>
        </div>
      )}
    </section>
  );
}
