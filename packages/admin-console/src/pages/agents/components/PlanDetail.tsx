import { useState } from 'react'
import { CheckCircle, XCircle, Loader2, Circle, ChevronDown, ChevronRight, ListTodo } from 'lucide-react'

import type { PlanTrace, PlanTask } from '../../../api/agent-conversations'

interface PlanDetailProps {
  plan: PlanTrace
}

function TaskStatusIcon({ status }: { status: string }) {
  switch (status) {
    case 'completed':
      return <CheckCircle className="w-4 h-4 text-green-400" />
    case 'in_progress':
      return <Loader2 className="w-4 h-4 text-blue-400 animate-spin" />
    case 'cancelled':
      return <XCircle className="w-4 h-4 text-red-400" />
    default:
      return <Circle className="w-4 h-4 text-zinc-500" />
  }
}

function PlanStatusBadge({ status }: { status?: string }) {
  if (!status) return null
  const colors: Record<string, string> = {
    active: 'bg-blue-500/20 text-blue-300',
    completed: 'bg-green-500/20 text-green-300',
    cancelled: 'bg-red-500/20 text-red-300',
  }
  return (
    <span className={`px-2 py-0.5 text-xs rounded-full ${colors[status] || 'bg-zinc-500/20 text-zinc-300'}`}>
      {status}
    </span>
  )
}

function TaskItem({ task }: { task: PlanTask }) {
  const [expanded, setExpanded] = useState(false)

  return (
    <div className="flex flex-col">
      <div
        className="flex items-center gap-2 py-1.5 px-2 rounded hover:bg-white/5 cursor-pointer"
        onClick={() => task.description && setExpanded(!expanded)}
      >
        <TaskStatusIcon status={task.status} />
        <span className={`text-sm flex-1 ${task.status === 'completed' ? 'text-zinc-400 line-through' : task.status === 'cancelled' ? 'text-zinc-500 line-through' : 'text-zinc-200'}`}>
          {task.title}
        </span>
        {task.description && (
          expanded
            ? <ChevronDown className="w-3 h-3 text-zinc-500" />
            : <ChevronRight className="w-3 h-3 text-zinc-500" />
        )}
      </div>
      {expanded && task.description && (
        <div className="ml-8 mb-1 text-xs text-zinc-400 bg-white/5 rounded p-2">
          {task.description}
        </div>
      )}
    </div>
  )
}

export default function PlanDetail({ plan }: PlanDetailProps) {
  return (
    <div className="bg-white/5 border border-white/10 rounded-lg overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-white/10">
        <ListTodo className="w-4 h-4 text-purple-400" />
        <span className="text-sm font-medium text-zinc-200">{plan.title || 'Plan'}</span>
        <PlanStatusBadge status={plan.status} />
      </div>
      {plan.tasks.length > 0 ? (
        <div className="p-2">
          {plan.tasks.map(task => (
            <TaskItem key={task.path} task={task} />
          ))}
        </div>
      ) : (
        <div className="p-3 text-xs text-zinc-500">No tasks</div>
      )}
    </div>
  )
}
