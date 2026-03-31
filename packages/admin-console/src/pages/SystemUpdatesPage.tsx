import { useEffect, useState } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import {
  RefreshCw,
  AlertTriangle,
  Check,
  FileType,
  FolderOpen,
  Package,
  ChevronDown,
  ChevronRight,
  ArrowRight,
  Loader2,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import {
  systemUpdatesApi,
  PendingUpdatesResponse,
  PendingUpdateInfo,
} from '../api/system-updates'
import { useToast, ToastContainer } from '../components/Toast'

export default function SystemUpdatesPage() {
  const { repo } = useParams<{ repo: string }>()
  const navigate = useNavigate()
  const [updates, setUpdates] = useState<PendingUpdatesResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [applying, setApplying] = useState(false)
  const [selectedUpdates, setSelectedUpdates] = useState<Set<string>>(new Set())
  const [expandedUpdates, setExpandedUpdates] = useState<Set<string>>(new Set())
  const [forceApply, setForceApply] = useState(false)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const tenant = 'default' // TODO: Get from context

  useEffect(() => {
    if (repo) {
      loadUpdates()
    }
  }, [repo])

  async function loadUpdates() {
    if (!repo) return
    try {
      setLoading(true)
      const response = await systemUpdatesApi.getPending(tenant, repo)
      setUpdates(response)
      // Select all non-breaking updates by default
      const nonBreaking = response.updates
        .filter((u) => !u.is_breaking)
        .map((u) => u.name)
      setSelectedUpdates(new Set(nonBreaking))
    } catch (err) {
      console.error('Failed to load updates:', err)
      showError('Failed to load system updates')
    } finally {
      setLoading(false)
    }
  }

  function toggleUpdate(name: string) {
    setSelectedUpdates((prev) => {
      const next = new Set(prev)
      if (next.has(name)) {
        next.delete(name)
      } else {
        next.add(name)
      }
      return next
    })
  }

  function toggleAllUpdates() {
    if (!updates) return
    if (selectedUpdates.size === updates.updates.length) {
      setSelectedUpdates(new Set())
    } else {
      setSelectedUpdates(new Set(updates.updates.map((u) => u.name)))
    }
  }

  function toggleExpandUpdate(name: string) {
    setExpandedUpdates((prev) => {
      const next = new Set(prev)
      if (next.has(name)) {
        next.delete(name)
      } else {
        next.add(name)
      }
      return next
    })
  }

  async function applySelectedUpdates() {
    if (!repo || selectedUpdates.size === 0) return

    const selectedList = Array.from(selectedUpdates)
    const hasBreaking = updates?.updates.some(
      (u) => selectedList.includes(u.name) && u.is_breaking
    )

    if (hasBreaking && !forceApply) {
      showError(
        'Selected updates contain breaking changes. Enable "Force apply" to proceed.'
      )
      return
    }

    try {
      setApplying(true)
      const response = await systemUpdatesApi.applySelected(
        tenant,
        repo,
        selectedList,
        forceApply
      )
      showSuccess(response.message)
      // Reload updates to reflect changes
      await loadUpdates()
      // If all updates applied, navigate back
      if (response.applied_count === selectedList.length) {
        // Small delay to show success message
        setTimeout(() => navigate(`/${repo}/settings`), 1500)
      }
    } catch (err: any) {
      console.error('Failed to apply updates:', err)
      showError(err.message || 'Failed to apply updates')
    } finally {
      setApplying(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-[400px]">
        <Loader2 className="w-8 h-8 text-primary-400 animate-spin" />
      </div>
    )
  }

  if (!updates || !updates.has_updates) {
    return (
      <div className="space-y-6">
        <div className="flex items-center justify-between">
          <h1 className="text-2xl font-bold text-white">System Updates</h1>
        </div>

        <GlassCard>
          <div className="text-center py-12">
            <Check className="w-16 h-16 text-green-400 mx-auto mb-4" />
            <h2 className="text-xl font-semibold text-white mb-2">
              All Up to Date
            </h2>
            <p className="text-white/60">
              Your repository has all the latest NodeTypes, Workspaces, and Packages.
            </p>
            <button
              onClick={() => navigate(`/${repo}/settings`)}
              className="mt-6 px-4 py-2 bg-primary-600 hover:bg-primary-500 text-white rounded-lg transition-colors"
            >
              Back to Settings
            </button>
          </div>
        </GlassCard>
        <ToastContainer toasts={toasts} onClose={closeToast} />
      </div>
    )
  }

  const nodeTypeUpdates = updates.updates.filter(
    (u) => u.resource_type === 'NodeType'
  )
  const workspaceUpdates = updates.updates.filter(
    (u) => u.resource_type === 'Workspace'
  )
  const packageUpdates = updates.updates.filter(
    (u) => u.resource_type === 'Package'
  )
  const hasBreakingSelected = updates.updates.some(
    (u) => selectedUpdates.has(u.name) && u.is_breaking
  )

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-white">System Updates</h1>
          <p className="text-white/60 mt-1">
            {updates.total_pending} update{updates.total_pending !== 1 ? 's' : ''}{' '}
            available
            {updates.breaking_count > 0 && (
              <span className="ml-2 text-red-400">
                ({updates.breaking_count} with breaking changes)
              </span>
            )}
          </p>
        </div>
        <button
          onClick={loadUpdates}
          disabled={loading}
          className="p-2 text-white/60 hover:text-white hover:bg-white/10 rounded-lg transition-colors"
          title="Refresh"
        >
          <RefreshCw className={`w-5 h-5 ${loading ? 'animate-spin' : ''}`} />
        </button>
      </div>

      {/* Breaking changes warning */}
      {updates.breaking_count > 0 && (
        <div className="bg-red-900/30 border border-red-500/50 rounded-lg p-4">
          <div className="flex items-start gap-3">
            <AlertTriangle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
            <div>
              <h3 className="text-red-300 font-semibold">
                Breaking Changes Detected
              </h3>
              <p className="text-red-200/80 text-sm mt-1">
                {updates.breaking_count} update
                {updates.breaking_count !== 1 ? 's' : ''} contain breaking
                changes that may affect existing data. Review carefully before
                applying.
              </p>
            </div>
          </div>
        </div>
      )}

      {/* NodeType Updates */}
      {nodeTypeUpdates.length > 0 && (
        <GlassCard className="space-y-4">
          <div className="flex items-center gap-3">
            <FileType className="w-5 h-5 text-blue-400" />
            <h2 className="text-lg font-semibold text-white">
              NodeType Updates ({nodeTypeUpdates.length})
            </h2>
          </div>

          <div className="space-y-2">
            {nodeTypeUpdates.map((update) => (
              <UpdateItem
                key={update.name}
                update={update}
                selected={selectedUpdates.has(update.name)}
                expanded={expandedUpdates.has(update.name)}
                onToggleSelect={() => toggleUpdate(update.name)}
                onToggleExpand={() => toggleExpandUpdate(update.name)}
              />
            ))}
          </div>
        </GlassCard>
      )}

      {/* Workspace Updates */}
      {workspaceUpdates.length > 0 && (
        <GlassCard className="space-y-4">
          <div className="flex items-center gap-3">
            <FolderOpen className="w-5 h-5 text-purple-400" />
            <h2 className="text-lg font-semibold text-white">
              Workspace Updates ({workspaceUpdates.length})
            </h2>
          </div>

          <div className="space-y-2">
            {workspaceUpdates.map((update) => (
              <UpdateItem
                key={update.name}
                update={update}
                selected={selectedUpdates.has(update.name)}
                expanded={expandedUpdates.has(update.name)}
                onToggleSelect={() => toggleUpdate(update.name)}
                onToggleExpand={() => toggleExpandUpdate(update.name)}
              />
            ))}
          </div>
        </GlassCard>
      )}

      {/* Package Updates */}
      {packageUpdates.length > 0 && (
        <GlassCard className="space-y-4">
          <div className="flex items-center gap-3">
            <Package className="w-5 h-5 text-cyan-400" />
            <h2 className="text-lg font-semibold text-white">
              Package Updates ({packageUpdates.length})
            </h2>
          </div>

          <div className="space-y-2">
            {packageUpdates.map((update) => (
              <UpdateItem
                key={update.name}
                update={update}
                selected={selectedUpdates.has(update.name)}
                expanded={expandedUpdates.has(update.name)}
                onToggleSelect={() => toggleUpdate(update.name)}
                onToggleExpand={() => toggleExpandUpdate(update.name)}
              />
            ))}
          </div>
        </GlassCard>
      )}

      {/* Actions */}
      <GlassCard className="space-y-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <label className="flex items-center gap-2 text-white/80">
              <input
                type="checkbox"
                checked={selectedUpdates.size === updates.updates.length}
                onChange={toggleAllUpdates}
                className="rounded border-white/30 bg-white/10 text-primary-500 focus:ring-primary-500"
              />
              Select All
            </label>

            {hasBreakingSelected && (
              <label className="flex items-center gap-2 text-red-300">
                <input
                  type="checkbox"
                  checked={forceApply}
                  onChange={(e) => setForceApply(e.target.checked)}
                  className="rounded border-red-500/30 bg-red-900/30 text-red-500 focus:ring-red-500"
                />
                Force apply (including breaking changes)
              </label>
            )}
          </div>

          <div className="flex items-center gap-3">
            <button
              onClick={() => navigate(`/${repo}/settings`)}
              className="px-4 py-2 text-white/60 hover:text-white hover:bg-white/10 rounded-lg transition-colors"
            >
              Cancel
            </button>

            <button
              onClick={applySelectedUpdates}
              disabled={applying || selectedUpdates.size === 0}
              className="flex items-center gap-2 px-4 py-2 bg-primary-600 hover:bg-primary-500 disabled:bg-primary-600/50 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
            >
              {applying ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Applying...
                </>
              ) : (
                <>
                  Apply Selected ({selectedUpdates.size})
                  <ArrowRight className="w-4 h-4" />
                </>
              )}
            </button>
          </div>
        </div>
      </GlassCard>

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}

interface UpdateItemProps {
  update: PendingUpdateInfo
  selected: boolean
  expanded: boolean
  onToggleSelect: () => void
  onToggleExpand: () => void
}

function UpdateItem({
  update,
  selected,
  expanded,
  onToggleSelect,
  onToggleExpand,
}: UpdateItemProps) {
  return (
    <div
      className={`rounded-lg border ${
        update.is_breaking
          ? 'border-red-500/30 bg-red-900/10'
          : 'border-white/10 bg-white/5'
      }`}
    >
      <div className="flex items-center gap-3 p-3">
        <input
          type="checkbox"
          checked={selected}
          onChange={onToggleSelect}
          className={`rounded ${
            update.is_breaking
              ? 'border-red-500/30 bg-red-900/30 text-red-500 focus:ring-red-500'
              : 'border-white/30 bg-white/10 text-primary-500 focus:ring-primary-500'
          }`}
        />

        <button
          onClick={onToggleExpand}
          className="text-white/60 hover:text-white transition-colors"
        >
          {expanded ? (
            <ChevronDown className="w-4 h-4" />
          ) : (
            <ChevronRight className="w-4 h-4" />
          )}
        </button>

        <div className="flex-1">
          <div className="flex items-center gap-2">
            <span className="font-medium text-white">{update.name}</span>
            {update.is_new && (
              <span className="px-2 py-0.5 text-xs bg-green-500/20 text-green-300 rounded">
                New
              </span>
            )}
            {update.is_breaking && (
              <span className="px-2 py-0.5 text-xs bg-red-500/20 text-red-300 rounded flex items-center gap-1">
                <AlertTriangle className="w-3 h-3" />
                Breaking
              </span>
            )}
          </div>
          <div className="text-sm text-white/60 mt-0.5">
            {update.old_version !== null
              ? `v${update.old_version} → v${update.new_version}`
              : `New (v${update.new_version})`}
          </div>
        </div>
      </div>

      {expanded && update.is_breaking && update.breaking_count > 0 && (
        <div className="border-t border-red-500/20 p-3 bg-red-900/5">
          <div className="text-sm text-red-300 font-medium mb-2">
            Breaking Changes ({update.breaking_count}):
          </div>
          <div className="text-sm text-red-200/70">
            This update contains {update.breaking_count} breaking change
            {update.breaking_count !== 1 ? 's' : ''}. Review the changes
            carefully before applying.
          </div>
        </div>
      )}
    </div>
  )
}
