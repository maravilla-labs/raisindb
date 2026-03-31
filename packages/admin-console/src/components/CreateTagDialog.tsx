import { useState, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { X, Tag, AlertCircle } from 'lucide-react'
import { tagsApi, branchesApi, type CreateTagRequest, type Branch } from '../api/branches'

interface CreateTagDialogProps {
  repoId: string
  onClose: () => void
  onSuccess: () => void
  /** Optional revision to tag (HLC format: "timestamp-counter") */
  defaultRevision?: string
}

export default function CreateTagDialog({
  repoId,
  onClose,
  onSuccess,
  defaultRevision
}: CreateTagDialogProps) {
  const [name, setName] = useState('')
  const [revision, setRevision] = useState<string>(defaultRevision || '0-0')
  const [message, setMessage] = useState('')
  const [createdBy, setCreatedBy] = useState('system')
  const [isProtected, setIsProtected] = useState(true) // Tags are typically protected
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [branches, setBranches] = useState<Branch[]>([])
  const [selectedBranch, setSelectedBranch] = useState<string>('main')

  useEffect(() => {
    loadBranches()
  }, [repoId])

  // Update revision when branches are loaded and no defaultRevision is set
  useEffect(() => {
    if (branches.length > 0 && !defaultRevision) {
      const branch = branches.find(b => b.name === selectedBranch) || branches[0]
      setRevision(branch.head)
    }
  }, [branches, selectedBranch, defaultRevision])

  async function loadBranches() {
    try {
      const data = await branchesApi.list(repoId)
      setBranches(data)
    } catch (error) {
      console.error('Failed to load branches:', error)
    }
  }

  function handleBranchChange(branchName: string) {
    setSelectedBranch(branchName)
    const branch = branches.find(b => b.name === branchName)
    if (branch) {
      setRevision(branch.head)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setError(null)
    setLoading(true)

    try {
      const data: CreateTagRequest = {
        name: name.trim(),
        revision,
        created_by: createdBy.trim() || 'system',
        message: message.trim() || undefined,
        protected: isProtected
      }

      await tagsApi.create(repoId, data)
      onSuccess()
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to create tag')
    } finally {
      setLoading(false)
    }
  }

  function validateName(value: string): boolean {
    // Tag name rules: alphanumeric, hyphens, underscores, dots (for semver)
    const nameRegex = /^[a-zA-Z0-9._-]+$/
    return nameRegex.test(value) && value.length > 0 && value.length <= 100
  }

  function validateRevision(value: string): boolean {
    // HLC format: "timestamp-counter" (e.g., "1762780281515-0")
    const hlcRegex = /^\d+-\d+$/
    return hlcRegex.test(value)
  }

  const isValid = validateName(name) && validateRevision(revision)

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl w-full max-w-lg">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-amber-500/20 rounded-lg">
              <Tag className="w-5 h-5 text-amber-400" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-white">Create New Tag</h2>
              <p className="text-sm text-gray-400">Create an immutable snapshot at a specific revision</p>
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
          {/* Tag Name */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Tag Name *
            </label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g., v1.0.0, release-2024-01, beta-1"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500"
              required
              autoFocus
            />
            {name && !isValid && (
              <p className="mt-1 text-xs text-red-400">
                Invalid name. Use only letters, numbers, dots, hyphens, and underscores.
              </p>
            )}
            <p className="mt-1 text-xs text-gray-500">
              Use semantic versioning (v1.0.0) or descriptive names
            </p>
          </div>

          {/* Source Branch */}
          {!defaultRevision && (
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Tag from branch
              </label>
              <select
                value={selectedBranch}
                onChange={(e) => handleBranchChange(e.target.value)}
                className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-amber-500"
              >
                {branches.map((branch) => (
                  <option key={branch.name} value={branch.name}>
                    {branch.name} (r{branch.head})
                  </option>
                ))}
              </select>
              <p className="mt-1 text-xs text-gray-500">
                Select which branch's HEAD to tag
              </p>
            </div>
          )}

          {/* Revision */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Revision *
            </label>
            <input
              type="text"
              value={revision}
              onChange={(e) => setRevision(e.target.value)}
              placeholder="e.g., 1762780281515-0"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500"
              required
            />
            {revision && !validateRevision(revision) && (
              <p className="mt-1 text-xs text-red-400">
                Invalid revision format. Use HLC format: timestamp-counter (e.g., 1762780281515-0)
              </p>
            )}
            <p className="mt-1 text-xs text-gray-500">
              The specific revision to create an immutable snapshot of (HLC format)
            </p>
          </div>

          {/* Message */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Release Notes / Message
            </label>
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="Describe this release or tag (optional)"
              rows={3}
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500 resize-none"
            />
            <p className="mt-1 text-xs text-gray-500">
              Optional: Add release notes, changelog, or description
            </p>
          </div>

          {/* Created By */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Created By
            </label>
            <input
              type="text"
              value={createdBy}
              onChange={(e) => setCreatedBy(e.target.value)}
              placeholder="Username or system"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-amber-500"
            />
          </div>

          {/* Protected Tag */}
          <div className="flex items-center gap-3">
            <input
              type="checkbox"
              id="protected"
              checked={isProtected}
              onChange={(e) => setIsProtected(e.target.checked)}
              className="w-4 h-4 bg-black/30 border border-white/20 rounded text-amber-500 focus:ring-2 focus:ring-amber-500"
            />
            <label htmlFor="protected" className="text-sm text-gray-300">
              Protected tag (prevent deletion)
            </label>
          </div>

          {/* Info Box */}
          <div className="flex items-start gap-2 p-3 bg-amber-500/10 border border-amber-500/20 rounded-lg">
            <AlertCircle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
            <div className="text-xs text-amber-300">
              <p className="font-medium mb-1">Tags are immutable snapshots</p>
              <p>Once created, tags cannot be moved or modified. They provide a permanent reference to a specific revision.</p>
            </div>
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
              disabled={!isValid || loading}
              className="px-6 py-2 bg-amber-500 hover:bg-amber-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors flex items-center gap-2"
            >
              {loading ? (
                <>
                  <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  Creating...
                </>
              ) : (
                <>
                  <Tag className="w-4 h-4" />
                  Create Tag
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
