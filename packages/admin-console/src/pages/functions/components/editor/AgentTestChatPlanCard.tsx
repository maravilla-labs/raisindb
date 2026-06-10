/**
 * Plan proposal / progress card for the agent Test Chat.
 *
 * Renders a `PlanProjection` (title, status, task list) with Approve / Reject
 * actions while the plan is pending approval. Reference implementation of the
 * SDK plan-approval contract (`ConversationStore.plans` + `approvePlan` /
 * `rejectPlan`) for custom chat UIs.
 */

import { useState } from 'react'
import {
  ListTodo,
  Loader2,
  Check,
  X,
  Circle,
  CircleDot,
  CheckCircle2,
  XCircle,
} from 'lucide-react'
import type { PlanProjection } from '../../../../api/agent-chat-plans'

const PLAN_STATUS_STYLES: Record<string, string> = {
  pending_approval: 'bg-yellow-500/20 text-yellow-300',
  in_progress: 'bg-blue-500/20 text-blue-300',
  completed: 'bg-green-500/20 text-green-300',
  cancelled: 'bg-red-500/20 text-red-300',
  failed: 'bg-red-500/20 text-red-300',
}

function TaskStatusIcon({ status }: { status: string }) {
  switch (status) {
    case 'completed':
      return <CheckCircle2 className="w-3.5 h-3.5 text-green-400 flex-shrink-0" />
    case 'in_progress':
      return <CircleDot className="w-3.5 h-3.5 text-blue-400 animate-pulse flex-shrink-0" />
    case 'failed':
    case 'cancelled':
      return <XCircle className="w-3.5 h-3.5 text-red-400 flex-shrink-0" />
    default:
      return <Circle className="w-3.5 h-3.5 text-zinc-500 flex-shrink-0" />
  }
}

interface PlanCardProps {
  plan: PlanProjection
  /** True while an approve/reject for this plan is in flight. */
  busy: boolean
  onApprove: (planPath: string) => void
  onReject: (planPath: string, feedback?: string) => void
}

export function AgentTestChatPlanCard({ plan, busy, onApprove, onReject }: PlanCardProps) {
  const [showRejectForm, setShowRejectForm] = useState(false)
  const [feedback, setFeedback] = useState('')

  const canAct = Boolean(plan.planPath) && plan.requiresApproval && plan.status === 'pending_approval'
  const statusStyle = PLAN_STATUS_STYLES[plan.status] ?? 'bg-zinc-500/20 text-zinc-300'

  return (
    <div
      className="border border-purple-500/30 bg-purple-500/5 rounded-lg p-3 space-y-2"
      data-testid="plan-card"
      data-plan-status={plan.status}
    >
      <div className="flex items-center gap-2">
        <ListTodo className="w-4 h-4 text-purple-400 flex-shrink-0" />
        <span className="text-sm font-medium text-zinc-200 flex-1 min-w-0 truncate">
          {plan.title}
        </span>
        <span className={`text-xs px-1.5 py-0.5 rounded ${statusStyle}`}>
          {plan.status.replace(/_/g, ' ')}
        </span>
      </div>

      {plan.tasks.length > 0 && (
        <ol className="space-y-1">
          {plan.tasks.map((task, index) => (
            <li
              key={task.taskId ?? `${index}-${task.title}`}
              className="flex items-start gap-2 text-xs text-zinc-300"
              data-testid="plan-task"
              data-task-status={task.status}
            >
              <TaskStatusIcon status={task.status} />
              <span className={task.status === 'completed' ? 'text-zinc-500 line-through' : ''}>
                {task.title}
              </span>
              {task.status === 'in_progress' && (
                <span className="text-blue-300/70">running…</span>
              )}
            </li>
          ))}
        </ol>
      )}

      {canAct && !showRejectForm && (
        <div className="flex gap-2 pt-1">
          <button
            onClick={() => plan.planPath && onApprove(plan.planPath)}
            disabled={busy}
            className="px-2.5 py-1 bg-green-500/20 text-green-300 rounded text-xs hover:bg-green-500/30 disabled:opacity-50 flex items-center gap-1.5"
            data-testid="plan-approve"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
            Approve
          </button>
          <button
            onClick={() => setShowRejectForm(true)}
            disabled={busy}
            className="px-2.5 py-1 bg-red-500/20 text-red-300 rounded text-xs hover:bg-red-500/30 disabled:opacity-50 flex items-center gap-1.5"
            data-testid="plan-reject"
          >
            <X className="w-3 h-3" />
            Reject
          </button>
        </div>
      )}

      {canAct && showRejectForm && (
        <div className="space-y-2 pt-1">
          <textarea
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            placeholder="Why are you rejecting this plan? (optional feedback for the agent)"
            rows={2}
            className="w-full px-2 py-1.5 bg-white/5 border border-white/10 rounded text-xs text-white placeholder-zinc-500 focus:border-red-500/50 focus:outline-none resize-none"
            data-testid="plan-reject-feedback"
          />
          <div className="flex gap-2">
            <button
              onClick={() => plan.planPath && onReject(plan.planPath, feedback || undefined)}
              disabled={busy}
              className="px-2.5 py-1 bg-red-500/20 text-red-300 rounded text-xs hover:bg-red-500/30 disabled:opacity-50 flex items-center gap-1.5"
              data-testid="plan-reject-confirm"
            >
              {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <X className="w-3 h-3" />}
              Send rejection
            </button>
            <button
              onClick={() => setShowRejectForm(false)}
              disabled={busy}
              className="px-2.5 py-1 text-zinc-400 rounded text-xs hover:bg-white/10"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
