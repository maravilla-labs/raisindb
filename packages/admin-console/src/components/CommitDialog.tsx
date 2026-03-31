import { useState } from 'react'
import { createPortal } from 'react-dom'
import { GitCommit, X, AlertCircle } from 'lucide-react'

interface CommitDialogProps {
  title: string
  action: string
  onCommit: (message: string, actor: string) => Promise<void>
  onClose: () => void
}

export default function CommitDialog({ title, action, onCommit, onClose }: CommitDialogProps) {
  const [message, setMessage] = useState('')
  const [actor, setActor] = useState('admin')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!message.trim()) {
      setError('Commit message is required')
      return
    }

    setError(null)
    setLoading(true)

    try {
      await onCommit(message.trim(), actor.trim())
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to commit changes')
    } finally {
      setLoading(false)
    }
  }

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4 overscroll-none">
      <div className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl w-full max-w-lg overscroll-contain">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-green-500/20 rounded-lg">
              <GitCommit className="w-5 h-5 text-green-400" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-white">{title}</h2>
              <p className="text-sm text-gray-400">{action}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-5 h-5 text-gray-400" />
          </button>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-6 space-y-5">
          {/* Commit Message */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Commit Message *
            </label>
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="Describe your changes..."
              rows={4}
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-green-500 resize-none"
              required
              autoFocus
            />
            <p className="mt-1 text-xs text-gray-500">
              A clear description of what changed and why
            </p>
          </div>

          {/* Actor */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Actor
            </label>
            <input
              type="text"
              value={actor}
              onChange={(e) => setActor(e.target.value)}
              placeholder="Username or system"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-green-500"
            />
            <p className="mt-1 text-xs text-gray-500">
              Who is making this change
            </p>
          </div>

          {/* Error Message */}
          {error && (
            <div className="flex items-center gap-2 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
              <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
              <p className="text-sm text-red-400">{error}</p>
            </div>
          )}

          {/* Actions */}
          <div className="flex items-center justify-end gap-3 pt-4 border-t border-white/10">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 text-gray-300 hover:text-white transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={loading}
              className="px-6 py-2 bg-green-500 hover:bg-green-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors flex items-center gap-2"
            >
              {loading ? (
                <>
                  <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  Committing...
                </>
              ) : (
                <>
                  <GitCommit className="w-4 h-4" />
                  Commit Changes
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body
  )
}
