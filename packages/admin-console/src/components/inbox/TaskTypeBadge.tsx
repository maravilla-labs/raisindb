/**
 * Task type badge
 *
 * Small chip describing the inbox task type (approval / input / review / action).
 * Reusable outside the inbox page (e.g. flow editor).
 */

import { CheckSquare, FormInput, Eye, Zap } from 'lucide-react'
import type { InboxTaskType } from '../../api/inbox'

const TYPE_CONFIG: Record<InboxTaskType, {
  icon: typeof CheckSquare
  color: string
  bg: string
  label: string
}> = {
  approval: { icon: CheckSquare, color: 'text-purple-400', bg: 'bg-purple-500/10', label: 'Approval' },
  input: { icon: FormInput, color: 'text-blue-400', bg: 'bg-blue-500/10', label: 'Input' },
  review: { icon: Eye, color: 'text-cyan-400', bg: 'bg-cyan-500/10', label: 'Review' },
  action: { icon: Zap, color: 'text-amber-400', bg: 'bg-amber-500/10', label: 'Action' },
}

interface TaskTypeBadgeProps {
  taskType: InboxTaskType
}

export default function TaskTypeBadge({ taskType }: TaskTypeBadgeProps) {
  const config = TYPE_CONFIG[taskType] || TYPE_CONFIG.action
  const Icon = config.icon

  return (
    <span className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${config.bg} ${config.color}`}>
      <Icon className="w-3 h-3" />
      {config.label}
    </span>
  )
}
