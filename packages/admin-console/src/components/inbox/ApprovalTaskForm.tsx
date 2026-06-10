/**
 * Approval task form
 *
 * Renders the task's options as styled action buttons plus an optional
 * comment. Submitting yields `{ action, comment? }`. Reusable outside the
 * inbox page (e.g. flow editor previews).
 */

import { useState } from 'react'
import type { InboxTaskOption } from '../../api/inbox'

interface ApprovalTaskFormProps {
  options: InboxTaskOption[]
  onSubmit: (response: { action: string; comment?: string }) => void
  busy?: boolean
}

const STYLE_CLASSES: Record<NonNullable<InboxTaskOption['style']>, string> = {
  success: 'bg-green-500/20 text-green-300 hover:bg-green-500/30 border border-green-500/30',
  danger: 'bg-red-500/20 text-red-300 hover:bg-red-500/30 border border-red-500/30',
  warning: 'bg-amber-500/20 text-amber-300 hover:bg-amber-500/30 border border-amber-500/30',
  default: 'bg-white/5 text-zinc-300 hover:bg-white/10 border border-white/10',
}

export default function ApprovalTaskForm({ options, onSubmit, busy }: ApprovalTaskFormProps) {
  const [comment, setComment] = useState('')

  const handleAction = (action: string) => {
    if (busy) return
    const trimmed = comment.trim()
    onSubmit(trimmed ? { action, comment: trimmed } : { action })
  }

  return (
    <div className="space-y-3">
      <div>
        <label className="block text-xs text-zinc-500 mb-1.5">Comment (optional)</label>
        <textarea
          value={comment}
          onChange={(e) => setComment(e.target.value)}
          rows={2}
          placeholder="Add a comment..."
          disabled={busy}
          className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-purple-500 disabled:opacity-50"
        />
      </div>
      <div className="flex flex-wrap gap-2">
        {options.map((option) => (
          <button
            key={option.value}
            onClick={() => handleAction(option.value)}
            disabled={busy}
            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed ${
              STYLE_CLASSES[option.style || 'default']
            }`}
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  )
}
