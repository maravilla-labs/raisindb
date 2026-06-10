/**
 * Assignee badge
 *
 * Shows the task assignee with a Bot icon for AI agents (paths starting with
 * `/agents/`) and a User icon for humans. Reusable outside the inbox page.
 */

import { Bot, User } from 'lucide-react'

interface AssigneeBadgeProps {
  /** Assignee path, e.g. `/users/alice` or `/agents/triage` */
  assignee: string
}

/** Last path segment as a friendly display name */
function displayName(assignee: string): string {
  const segments = assignee.split('/').filter(Boolean)
  return segments[segments.length - 1] || assignee
}

export default function AssigneeBadge({ assignee }: AssigneeBadgeProps) {
  const isAgent = assignee.startsWith('/agents/')
  const Icon = isAgent ? Bot : User

  return (
    <span
      className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${
        isAgent ? 'bg-violet-500/10 text-violet-400' : 'bg-white/5 text-zinc-300'
      }`}
      title={assignee}
    >
      <Icon className="w-3 h-3 flex-shrink-0" />
      <span className="truncate max-w-[160px]">{displayName(assignee)}</span>
    </span>
  )
}
