import { useState, useEffect } from 'react'
import {
  Eye,
  Loader2,
  XCircle,
  AlertTriangle,
  Plus,
  RefreshCw,
  SkipForward,
  FileCode,
  Folder,
  Box,
  FileText,
  Image,
  Package,
  ChevronDown,
  ChevronRight,
  PlayCircle,
} from 'lucide-react'
import { packagesApi, type DryRunResponse, type DryRunSummary, type InstallMode } from '../api/packages'

interface DryRunDialogProps {
  open: boolean
  repo: string
  packageName: string
  packageTitle?: string
  branch?: string
  mode: InstallMode
  onClose: () => void
  onProceed: () => void
}

type LoadingState = 'idle' | 'loading' | 'success' | 'error'

// Get icon and color for different action types
function getActionStyle(action: string) {
  switch (action) {
    case 'create':
      return { icon: Plus, color: 'text-green-400', bg: 'bg-green-400/10' }
    case 'update':
      return { icon: RefreshCw, color: 'text-blue-400', bg: 'bg-blue-400/10' }
    case 'skip':
      return { icon: SkipForward, color: 'text-zinc-400', bg: 'bg-zinc-400/10' }
    default:
      return { icon: FileText, color: 'text-zinc-400', bg: 'bg-zinc-400/10' }
  }
}

// Get icon for different categories
function getCategoryIcon(category: string) {
  switch (category) {
    case 'node_type':
      return FileCode
    case 'workspace':
      return Folder
    case 'content':
      return FileText
    case 'binary':
      return Image
    case 'archetype':
    case 'element_type':
      return Box
    case 'package_asset':
      return Package
    default:
      return FileText
  }
}

// Summary card component
function SummaryCard({
  title,
  icon: Icon,
  counts,
}: {
  title: string
  icon: React.ElementType
  counts: { create: number; update: number; skip: number }
}) {
  const total = counts.create + counts.update + counts.skip
  if (total === 0) return null

  return (
    <div className="bg-white/5 rounded-lg p-3 border border-white/10">
      <div className="flex items-center gap-2 mb-2">
        <Icon className="w-4 h-4 text-primary-400" />
        <span className="text-sm font-medium text-white">{title}</span>
        <span className="text-xs text-zinc-500">({total})</span>
      </div>
      <div className="flex gap-3 text-xs">
        {counts.create > 0 && (
          <span className="flex items-center gap-1 text-green-400">
            <Plus className="w-3 h-3" />
            {counts.create} new
          </span>
        )}
        {counts.update > 0 && (
          <span className="flex items-center gap-1 text-blue-400">
            <RefreshCw className="w-3 h-3" />
            {counts.update} update
          </span>
        )}
        {counts.skip > 0 && (
          <span className="flex items-center gap-1 text-zinc-400">
            <SkipForward className="w-3 h-3" />
            {counts.skip} skip
          </span>
        )}
      </div>
    </div>
  )
}

export default function DryRunDialog({
  open,
  repo,
  packageName,
  packageTitle,
  branch = 'main',
  mode,
  onClose,
  onProceed,
}: DryRunDialogProps) {
  const [loadingState, setLoadingState] = useState<LoadingState>('idle')
  const [result, setResult] = useState<DryRunResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [expandedCategories, setExpandedCategories] = useState<Set<string>>(new Set())

  // Fetch dry run results when dialog opens
  useEffect(() => {
    if (open) {
      setLoadingState('loading')
      setResult(null)
      setError(null)
      setExpandedCategories(new Set())

      packagesApi
        .dryRunPackage(repo, packageName, { mode, branch })
        .then((data) => {
          setResult(data)
          setLoadingState('success')
          // Auto-expand categories with actions
          const nonEmptyCategories = new Set<string>()
          data.logs.forEach((log) => {
            if (log.action !== 'info') {
              nonEmptyCategories.add(log.category)
            }
          })
          setExpandedCategories(nonEmptyCategories)
        })
        .catch((err) => {
          setError(err instanceof Error ? err.message : 'Failed to run dry run preview')
          setLoadingState('error')
        })
    }
  }, [open, repo, packageName, mode, branch])

  function toggleCategory(category: string) {
    setExpandedCategories((prev) => {
      const next = new Set(prev)
      if (next.has(category)) {
        next.delete(category)
      } else {
        next.add(category)
      }
      return next
    })
  }

  // Group logs by category
  function getGroupedLogs() {
    if (!result) return {}
    const groups: Record<string, typeof result.logs> = {}
    result.logs.forEach((log) => {
      if (log.action === 'info') return // Skip info messages
      if (!groups[log.category]) {
        groups[log.category] = []
      }
      groups[log.category].push(log)
    })
    return groups
  }

  // Get total counts across all categories
  function getTotalCounts(summary: DryRunSummary) {
    const categories = [
      summary.node_types,
      summary.archetypes,
      summary.element_types,
      summary.workspaces,
      summary.content_nodes,
      summary.binary_files,
      summary.package_assets,
    ]

    return categories.reduce(
      (acc, cat) => ({
        create: acc.create + cat.create,
        update: acc.update + cat.update,
        skip: acc.skip + cat.skip,
      }),
      { create: 0, update: 0, skip: 0 }
    )
  }

  if (!open) return null

  const groupedLogs = getGroupedLogs()
  const totalCounts = result ? getTotalCounts(result.summary) : { create: 0, update: 0, skip: 0 }
  const totalActions = totalCounts.create + totalCounts.update + totalCounts.skip

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={loadingState !== 'loading' ? onClose : undefined}
      />

      {/* Dialog */}
      <div className="relative w-full max-w-3xl max-h-[85vh] flex flex-col bg-zinc-900 border border-white/10 rounded-xl shadow-2xl">
        {/* Header */}
        <div className="flex items-center gap-3 px-6 py-4 border-b border-white/10 flex-shrink-0">
          <Eye className="w-6 h-6 text-primary-400" />
          <div className="flex-1">
            <h2 className="text-xl font-semibold text-white">Installation Preview</h2>
            <p className="text-sm text-zinc-400">
              Preview what would happen when installing{' '}
              <span className="text-white font-medium">{packageTitle || packageName}</span>
              {' '}with <span className="text-primary-400 font-medium">{mode.toUpperCase()}</span> mode
            </p>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {loadingState === 'loading' && (
            <div className="py-12 text-center">
              <Loader2 className="w-12 h-12 text-primary-400 animate-spin mx-auto mb-4" />
              <p className="text-white font-medium">Analyzing package...</p>
              <p className="text-sm text-zinc-400 mt-1">
                Simulating installation to preview changes
              </p>
            </div>
          )}

          {loadingState === 'error' && (
            <div className="py-12 text-center">
              <XCircle className="w-12 h-12 text-red-400 mx-auto mb-4" />
              <p className="text-white font-medium mb-2">Preview failed</p>
              {error && (
                <div className="flex items-start gap-2 p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-left max-w-md mx-auto">
                  <AlertTriangle className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" />
                  <p className="text-sm text-red-300">{error}</p>
                </div>
              )}
            </div>
          )}

          {loadingState === 'success' && result && (
            <div className="space-y-6">
              {/* Total Summary */}
              <div className="bg-white/5 rounded-lg p-4 border border-white/10">
                <h3 className="text-sm font-medium text-zinc-400 mb-3">Summary</h3>
                <div className="flex items-center gap-6">
                  <div className="flex items-center gap-2">
                    <div className="w-3 h-3 rounded-full bg-green-400" />
                    <span className="text-white font-medium">{totalCounts.create}</span>
                    <span className="text-zinc-400 text-sm">to create</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <div className="w-3 h-3 rounded-full bg-blue-400" />
                    <span className="text-white font-medium">{totalCounts.update}</span>
                    <span className="text-zinc-400 text-sm">to update</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <div className="w-3 h-3 rounded-full bg-zinc-400" />
                    <span className="text-white font-medium">{totalCounts.skip}</span>
                    <span className="text-zinc-400 text-sm">to skip</span>
                  </div>
                </div>
              </div>

              {/* Category Summary Cards */}
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
                <SummaryCard
                  title="Node Types"
                  icon={FileCode}
                  counts={result.summary.node_types}
                />
                <SummaryCard
                  title="Archetypes"
                  icon={Box}
                  counts={result.summary.archetypes}
                />
                <SummaryCard
                  title="Element Types"
                  icon={Box}
                  counts={result.summary.element_types}
                />
                <SummaryCard
                  title="Workspaces"
                  icon={Folder}
                  counts={result.summary.workspaces}
                />
                <SummaryCard
                  title="Content Nodes"
                  icon={FileText}
                  counts={result.summary.content_nodes}
                />
                <SummaryCard
                  title="Binary Files"
                  icon={Image}
                  counts={result.summary.binary_files}
                />
                <SummaryCard
                  title="Package Assets"
                  icon={Package}
                  counts={result.summary.package_assets}
                />
              </div>

              {/* Detailed Logs */}
              {totalActions > 0 && (
                <div className="space-y-2">
                  <h3 className="text-sm font-medium text-zinc-400">Details</h3>
                  <div className="border border-white/10 rounded-lg overflow-hidden">
                    {Object.entries(groupedLogs).map(([category, logs]) => {
                      const CategoryIcon = getCategoryIcon(category)
                      const isExpanded = expandedCategories.has(category)

                      return (
                        <div key={category} className="border-b border-white/10 last:border-b-0">
                          <button
                            onClick={() => toggleCategory(category)}
                            className="w-full flex items-center gap-2 px-4 py-3 bg-white/5 hover:bg-white/10 transition-colors text-left"
                          >
                            {isExpanded ? (
                              <ChevronDown className="w-4 h-4 text-zinc-400" />
                            ) : (
                              <ChevronRight className="w-4 h-4 text-zinc-400" />
                            )}
                            <CategoryIcon className="w-4 h-4 text-primary-400" />
                            <span className="text-white font-medium capitalize">
                              {category.replace(/_/g, ' ')}
                            </span>
                            <span className="text-xs text-zinc-500">({logs.length})</span>
                          </button>

                          {isExpanded && (
                            <div className="divide-y divide-white/5">
                              {logs.map((log, idx) => {
                                const { icon: ActionIcon, color, bg } = getActionStyle(log.action)

                                return (
                                  <div
                                    key={`${log.path}-${idx}`}
                                    className="flex items-start gap-3 px-4 py-2 pl-10 hover:bg-white/5"
                                  >
                                    <div className={`p-1 rounded ${bg}`}>
                                      <ActionIcon className={`w-3 h-3 ${color}`} />
                                    </div>
                                    <div className="flex-1 min-w-0">
                                      <p className="text-sm text-white font-mono truncate">
                                        {log.path}
                                      </p>
                                      <p className="text-xs text-zinc-500">{log.message}</p>
                                    </div>
                                  </div>
                                )
                              })}
                            </div>
                          )}
                        </div>
                      )
                    })}
                  </div>
                </div>
              )}

              {totalActions === 0 && (
                <div className="text-center py-8">
                  <Package className="w-12 h-12 text-zinc-500 mx-auto mb-3" />
                  <p className="text-zinc-400">
                    No changes will be made with the current mode.
                  </p>
                  <p className="text-sm text-zinc-500 mt-1">
                    Try a different install mode to see potential changes.
                  </p>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-between gap-3 px-6 py-4 border-t border-white/10 flex-shrink-0">
          <div className="text-sm text-zinc-500">
            {loadingState === 'success' && result && (
              <span>
                Package: <span className="text-zinc-400">{result.package_name}</span>{' '}
                v<span className="text-zinc-400">{result.package_version}</span>
              </span>
            )}
          </div>
          <div className="flex gap-3">
            <button
              onClick={onClose}
              className="px-4 py-2 text-zinc-400 hover:text-white transition-colors"
            >
              Cancel
            </button>
            {loadingState === 'success' && totalActions > 0 && (
              <button
                onClick={() => {
                  onClose()
                  onProceed()
                }}
                className="flex items-center gap-2 px-4 py-2 bg-green-500 hover:bg-green-600 text-white rounded-lg transition-colors"
              >
                <PlayCircle className="w-4 h-4" />
                Proceed with Install
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
