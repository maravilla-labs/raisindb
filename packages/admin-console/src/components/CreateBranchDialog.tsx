import { useState, useEffect } from 'react'
import { X, GitBranch, AlertCircle } from 'lucide-react'
import { branchesApi, type CreateBranchRequest, type Branch } from '../api/branches'

interface CreateBranchDialogProps {
  repoId: string
  onClose: () => void
  onSuccess: () => void
  /** Optional source branch to fork from */
  sourceBranch?: Branch
}

export default function CreateBranchDialog({
  repoId,
  onClose,
  onSuccess,
  sourceBranch
}: CreateBranchDialogProps) {
  const [name, setName] = useState('')
  const [fromRevision, setFromRevision] = useState<string | undefined>(
    sourceBranch?.head
  )
  const [createdBy, setCreatedBy] = useState('system')
  const [isProtected, setIsProtected] = useState(false)
  const [includeRevisionHistory, setIncludeRevisionHistory] = useState(true)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [branches, setBranches] = useState<Branch[]>([])
  const [selectedSourceBranch, setSelectedSourceBranch] = useState<string>(
    sourceBranch?.name || 'main'
  )

  useEffect(() => {
    loadBranches()
  }, [repoId])

  // Update fromRevision when branches are loaded or selectedSourceBranch changes
  useEffect(() => {
    if (branches.length > 0 && selectedSourceBranch) {
      const branch = branches.find(b => b.name === selectedSourceBranch)
      if (branch) {
        setFromRevision(branch.head)
      }
    }
  }, [branches, selectedSourceBranch])

  async function loadBranches() {
    try {
      const data = await branchesApi.list(repoId)
      setBranches(data)
    } catch (error) {
      console.error('Failed to load branches:', error)
    }
  }

  function handleSourceBranchChange(branchName: string) {
    setSelectedSourceBranch(branchName)
    const branch = branches.find(b => b.name === branchName)
    if (branch) {
      setFromRevision(branch.head)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setError(null)
    setLoading(true)

    try {
      const data: CreateBranchRequest = {
        name: name.trim(),
        from_revision: fromRevision,
        upstream_branch: selectedSourceBranch, // Track source branch for divergence comparison
        created_by: createdBy.trim() || 'system',
        protected: isProtected,
        include_revision_history: includeRevisionHistory
      }

      await branchesApi.create(repoId, data)
      onSuccess()
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to create branch')
    } finally {
      setLoading(false)
    }
  }

  function validateName(value: string): boolean {
    // Branch name rules: alphanumeric, hyphens, underscores, slashes
    const nameRegex = /^[a-zA-Z0-9/_-]+$/
    return nameRegex.test(value) && value.length > 0 && value.length <= 100
  }

  const isValid = validateName(name)

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl w-full max-w-lg">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-primary-500/20 rounded-lg">
              <GitBranch className="w-5 h-5 text-primary-400" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-white">Create New Branch</h2>
              <p className="text-sm text-gray-400">Fork from an existing branch or revision</p>
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
          {/* Branch Name */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Branch Name *
            </label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g., feature/new-layout, develop, hotfix/bug-123"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
              required
              autoFocus
            />
            {name && !isValid && (
              <p className="mt-1 text-xs text-red-400">
                Invalid name. Use only letters, numbers, hyphens, underscores, and slashes.
              </p>
            )}
            <p className="mt-1 text-xs text-gray-500">
              Use descriptive names like feature/*, bugfix/*, or release/*
            </p>
          </div>

          {/* Source Branch */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Create from
            </label>
            <select
              value={selectedSourceBranch}
              onChange={(e) => handleSourceBranchChange(e.target.value)}
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
            >
              {branches.map((branch) => (
                <option key={branch.name} value={branch.name}>
                  {branch.name} (r{branch.head})
                </option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500">
              The new branch will start from this branch's HEAD revision
            </p>
          </div>

          {/* Revision Override (Advanced) */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Revision (Optional)
            </label>
            <input
              type="text"
              value={fromRevision || ''}
              onChange={(e) => setFromRevision(e.target.value || undefined)}
              placeholder="Leave empty to use latest (e.g., 1762780281515-0)"
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
            />
            <p className="mt-1 text-xs text-gray-500">
              Advanced: Specify a specific revision number to fork from
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
              className="w-full px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
            />
          </div>

          {/* Protected Branch */}
          <div className="flex items-center gap-3">
            <input
              type="checkbox"
              id="protected"
              checked={isProtected}
              onChange={(e) => setIsProtected(e.target.checked)}
              className="w-4 h-4 bg-black/30 border border-white/20 rounded text-primary-500 focus:ring-2 focus:ring-primary-500"
            />
            <label htmlFor="protected" className="text-sm text-gray-300">
              Protected branch (prevent deletion and force pushes)
            </label>
          </div>

          {/* Include Revision History */}
          <div className="flex items-start gap-3">
            <input
              type="checkbox"
              id="includeRevisionHistory"
              checked={includeRevisionHistory}
              onChange={(e) => setIncludeRevisionHistory(e.target.checked)}
              className="w-4 h-4 mt-0.5 bg-black/30 border border-white/20 rounded text-primary-500 focus:ring-2 focus:ring-primary-500"
            />
            <div>
              <label htmlFor="includeRevisionHistory" className="text-sm text-gray-300">
                Include revision history
              </label>
              <p className="text-xs text-gray-500 mt-0.5">
                Copy commit history from source branch. History is copied in the background.
              </p>
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
              className="px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors flex items-center gap-2"
            >
              {loading ? (
                <>
                  <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  Creating...
                </>
              ) : (
                <>
                  <GitBranch className="w-4 h-4" />
                  Create Branch
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
