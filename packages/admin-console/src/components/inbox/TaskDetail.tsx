/**
 * Task detail
 *
 * Full view of an inbox task: badges, description, due date (with overdue
 * highlighting), escalation banner, the type-specific completion form for
 * pending tasks, and the recorded response for completed ones.
 *
 * Reusable outside the inbox page (e.g. flow editor).
 */

import { Link, useParams } from 'react-router-dom'
import { AlertTriangle, Clock, ExternalLink, CheckCircle } from 'lucide-react'
import type { InboxTask } from '../../api/inbox'
import TaskTypeBadge from './TaskTypeBadge'
import PriorityBadge from './PriorityBadge'
import AssigneeBadge from './AssigneeBadge'
import ApprovalTaskForm from './ApprovalTaskForm'
import InputTaskForm from './InputTaskForm'
import GenericTaskForm from './GenericTaskForm'
import MarkdownRenderer from '../MarkdownRenderer'

interface TaskDetailProps {
  task: InboxTask
  onComplete: (response: Record<string, unknown>) => Promise<void>
  busy?: boolean
}

export function isTaskOverdue(task: Pick<InboxTask, 'due_at' | 'status'>): boolean {
  return Boolean(
    task.due_at && task.status === 'pending' && new Date(task.due_at).getTime() < Date.now()
  )
}

export default function TaskDetail({ task, onComplete, busy }: TaskDetailProps) {
  const { repo } = useParams<{ repo: string }>()
  const overdue = isTaskOverdue(task)
  const isPending = task.status === 'pending'

  // Review/action tasks render the description inside GenericTaskForm
  const showDescription = task.description && (task.task_type === 'approval' || task.task_type === 'input' || !isPending)

  return (
    <div className="space-y-4">
      {/* Title + badges */}
      <div>
        <h2 className="text-lg font-semibold text-white mb-2">{task.title}</h2>
        <div className="flex flex-wrap items-center gap-2">
          <TaskTypeBadge taskType={task.task_type} />
          <PriorityBadge priority={task.priority} />
          <AssigneeBadge assignee={task.assignee} />
          {task.due_at && (
            <span
              className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${
                overdue ? 'bg-red-500/10 text-red-400' : 'bg-white/5 text-zinc-400'
              }`}
            >
              <Clock className="w-3 h-3" />
              {overdue ? 'Overdue: ' : 'Due '}
              {new Date(task.due_at).toLocaleString()}
            </span>
          )}
        </div>
      </div>

      {/* Escalation banner */}
      {task.escalated_from && (
        <div className="p-3 bg-amber-500/10 border border-amber-500/20 rounded-lg">
          <div className="flex items-center gap-2 text-sm font-medium text-amber-300 mb-1">
            <AlertTriangle className="w-4 h-4" />
            Escalated from <span className="font-mono">{task.escalated_from}</span>
          </div>
          {task.escalation_reason && (
            <div className="text-sm text-amber-200/80">{task.escalation_reason}</div>
          )}
        </div>
      )}

      {/* Description */}
      {showDescription && task.description && (
        <div className="text-sm text-zinc-300">
          <MarkdownRenderer content={task.description} />
        </div>
      )}

      {/* Flow instance link */}
      {task.flow_instance_id && repo && (
        <Link
          to={`/${repo}/flows`}
          className="inline-flex items-center gap-1.5 text-sm text-purple-400 hover:text-purple-300 transition-colors"
        >
          <ExternalLink className="w-4 h-4" />
          Flow instance {task.flow_instance_id.slice(0, 8)}...
          {task.step_id && <span className="text-zinc-500">@ {task.step_id}</span>}
        </Link>
      )}

      {/* Pending: completion form by task type */}
      {isPending ? (
        <div className="pt-2 border-t border-white/10">
          {task.task_type === 'approval' ? (
            <ApprovalTaskForm
              options={task.options || []}
              onSubmit={(response) => void onComplete(response)}
              busy={busy}
            />
          ) : task.task_type === 'input' ? (
            <InputTaskForm
              schema={task.input_schema || {}}
              onSubmit={(values) => void onComplete(values)}
              busy={busy}
            />
          ) : (
            <GenericTaskForm
              description={task.description}
              onSubmit={(response) => void onComplete(response)}
              busy={busy}
            />
          )}
        </div>
      ) : (
        /* Non-pending: completed state */
        <div className="pt-2 border-t border-white/10 space-y-3">
          <div
            className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium capitalize ${
              task.status === 'completed'
                ? 'bg-green-500/10 text-green-400'
                : 'bg-zinc-500/10 text-zinc-400'
            }`}
          >
            <CheckCircle className="w-3 h-3" />
            {task.status}
          </div>

          <div className="grid grid-cols-2 gap-4">
            {task.completed_by && (
              <div>
                <div className="text-xs text-zinc-500 mb-1">Completed by</div>
                <AssigneeBadge assignee={task.completed_by} />
              </div>
            )}
            {task.responded_at && (
              <div>
                <div className="text-xs text-zinc-500 mb-1">Responded at</div>
                <div className="text-sm text-zinc-300">
                  {new Date(task.responded_at).toLocaleString()}
                </div>
              </div>
            )}
          </div>

          {task.response && Object.keys(task.response).length > 0 && (
            <div>
              <div className="text-xs text-zinc-500 mb-2">Response</div>
              <pre className="text-xs text-zinc-400 bg-black/30 p-3 rounded-lg overflow-auto max-h-48">
                {JSON.stringify(task.response, null, 2)}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
