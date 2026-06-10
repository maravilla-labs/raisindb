/**
 * Generic task form (review / action tasks)
 *
 * Shows the task description (markdown), an optional comment, and a
 * "Complete" button. Submitting yields `{ acknowledged: true, comment? }`.
 * Reusable outside the inbox page.
 */

import { useState } from 'react'
import { CheckCircle } from 'lucide-react'
import MarkdownRenderer from '../MarkdownRenderer'

interface GenericTaskFormProps {
  description?: string
  onSubmit: (response: { acknowledged: true; comment?: string }) => void
  busy?: boolean
}

export default function GenericTaskForm({ description, onSubmit, busy }: GenericTaskFormProps) {
  const [comment, setComment] = useState('')

  const handleSubmit = () => {
    if (busy) return
    const trimmed = comment.trim()
    onSubmit(trimmed ? { acknowledged: true, comment: trimmed } : { acknowledged: true })
  }

  return (
    <div className="space-y-3">
      {description && (
        <div className="text-sm text-zinc-300">
          <MarkdownRenderer content={description} />
        </div>
      )}
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
      <button
        onClick={handleSubmit}
        disabled={busy}
        className="px-4 py-2 bg-green-500/20 text-green-300 hover:bg-green-500/30 border border-green-500/30 rounded-lg text-sm font-medium transition-colors flex items-center gap-2 disabled:opacity-50 disabled:cursor-not-allowed"
      >
        <CheckCircle className="w-4 h-4" />
        {busy ? 'Completing...' : 'Complete'}
      </button>
    </div>
  )
}
