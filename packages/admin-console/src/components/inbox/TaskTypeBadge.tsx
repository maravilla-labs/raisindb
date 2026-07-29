/**
 * Task type badge
 *
 * Small chip describing the inbox task type. The canonical types
 * (approval / input / review / action) get their own icon and colour; an
 * application-defined type falls back to a neutral chip showing the type's
 * own name, rather than being mislabelled as one of the four.
 * Reusable outside the inbox page (e.g. flow editor).
 */

import { CheckSquare, FormInput, Eye, Zap, Tag } from 'lucide-react'
import { formatLabel } from '../../utils/propertySchema'
import type { CanonicalTaskType, InboxTaskType } from '../../api/inbox'

const TYPE_CONFIG: Record<CanonicalTaskType, {
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
  const config = TYPE_CONFIG[taskType as CanonicalTaskType] ?? {
    icon: Tag,
    color: 'text-slate-300',
    bg: 'bg-slate-500/10',
    label: formatLabel(taskType),
  }
  const Icon = config.icon

  return (
    <span className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${config.bg} ${config.color}`}>
      <Icon className="w-3 h-3" />
      {config.label}
    </span>
  )
}
