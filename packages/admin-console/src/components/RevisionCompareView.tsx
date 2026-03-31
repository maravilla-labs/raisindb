import { useState, useEffect } from 'react'
import { X, Plus, Edit, Trash2, GitCompare, ChevronDown, ChevronRight, Loader2, Globe } from 'lucide-react'
import { Node as NodeType, nodesApi } from '../api/nodes'
import { revisionsApi } from '../api/revisions'
import { computePropertyDiff, formatPropertyPath, formatDiffValue, PropertyDiff } from '../utils/propertyDiff'

interface RevisionCompareViewProps {
  repo: string
  branch: string
  workspace: string
  fromRevision: string  // HLC format: "timestamp-counter"
  toRevision: string  // HLC format: "timestamp-counter"
  onClose: () => void
}

interface NodeComparison {
  nodeId: string
  nodePath?: string
  nodeType?: string
  operation: 'added' | 'deleted' | 'modified' | 'unchanged'
  translationLocale?: string
  fromNode?: NodeType
  toNode?: NodeType
  propertyDiffs?: PropertyDiff[]
}

export default function RevisionCompareView({
  repo,
  branch,
  workspace,
  fromRevision,
  toRevision,
  onClose,
}: RevisionCompareViewProps) {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [comparisons, setComparisons] = useState<NodeComparison[]>([])
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())

  useEffect(() => {
    loadComparison()
  }, [repo, branch, workspace, fromRevision, toRevision])

  async function loadComparison() {
    setLoading(true)
    setError(null)

    try {
      // Fetch all changes between the two revisions
      const { changes } = await revisionsApi.compare(repo, fromRevision, toRevision)

      // Build a map tracking the latest operation for each node
      const nodeMap = new Map<string, NodeComparison>()

      // Process all changes - later changes override earlier ones
      for (const change of changes) {
        const existing = nodeMap.get(change.node_id)

        if (!existing) {
          // First time seeing this node
          if (change.operation === 'added') {
            nodeMap.set(change.node_id, {
              nodeId: change.node_id,
              nodePath: change.path,
              nodeType: change.node_type,
              operation: 'added',
              translationLocale: change.translation_locale,
            })
          } else if (change.operation === 'deleted') {
            nodeMap.set(change.node_id, {
              nodeId: change.node_id,
              nodePath: change.path,
              nodeType: change.node_type,
              operation: 'deleted',
              translationLocale: change.translation_locale,
            })
          } else if (change.operation === 'modified') {
            nodeMap.set(change.node_id, {
              nodeId: change.node_id,
              nodePath: change.path,
              nodeType: change.node_type,
              operation: 'modified',
              translationLocale: change.translation_locale,
            })
          }
        } else {
          // We've seen this node before - update operation
          if (change.operation === 'deleted') {
            // If node was added then deleted, remove it entirely
            if (existing.operation === 'added') {
              nodeMap.delete(change.node_id)
            } else {
              existing.operation = 'deleted'
            }
          } else if (change.operation === 'added') {
            // This shouldn't happen (node can't be added twice)
            // but if it does, treat as modified
            if (existing.operation === 'deleted') {
              existing.operation = 'modified'
            }
          } else if (change.operation === 'modified') {
            // Modified operations compound
            if (existing.operation === 'added') {
              // Still an add, just with more changes
              existing.operation = 'added'
            } else if (existing.operation === 'deleted') {
              // Modified after delete? Shouldn't happen
              existing.operation = 'modified'
            } else {
              existing.operation = 'modified'
            }
          }

          // Update path, type, and translation_locale if available
          existing.nodePath = change.path || existing.nodePath
          existing.nodeType = change.node_type || existing.nodeType
          existing.translationLocale = change.translation_locale || existing.translationLocale
        }
      }

      // Fetch node details for all nodes to compute property diffs
      const nodeComparisons = Array.from(nodeMap.values())

      await Promise.all(
        nodeComparisons.map(async (comparison) => {
          if (comparison.operation === 'modified') {
            try {
              const [fromNode, toNode] = await Promise.all([
                nodesApi.getByIdAtRevision(repo, branch, workspace, comparison.nodeId, fromRevision),
                nodesApi.getByIdAtRevision(repo, branch, workspace, comparison.nodeId, toRevision),
              ])

              comparison.fromNode = fromNode
              comparison.toNode = toNode

              // Compute property-level diffs
              comparison.propertyDiffs = computePropertyDiff(
                fromNode.properties || {},
                toNode.properties || {}
              )
            } catch (err) {
              console.error(`Failed to fetch node ${comparison.nodeId}:`, err)
            }
          } else if (comparison.operation === 'added') {
            // Fetch the node from toRevision
            try {
              const toNode = await nodesApi.getByIdAtRevision(repo, branch, workspace, comparison.nodeId, toRevision)
              comparison.toNode = toNode
            } catch (err) {
              console.error(`Failed to fetch added node ${comparison.nodeId}:`, err)
            }
          } else if (comparison.operation === 'deleted') {
            // Fetch the node from fromRevision
            try {
              const fromNode = await nodesApi.getByIdAtRevision(repo, branch, workspace, comparison.nodeId, fromRevision)
              comparison.fromNode = fromNode
            } catch (err) {
              console.error(`Failed to fetch deleted node ${comparison.nodeId}:`, err)
            }
          }
        })
      )

      // Sort: added first, then modified, then deleted
      const operationOrder = { added: 0, modified: 1, deleted: 2, unchanged: 3 }
      nodeComparisons.sort((a, b) => {
        const orderDiff = operationOrder[a.operation] - operationOrder[b.operation]
        if (orderDiff !== 0) return orderDiff
        // Within same operation, sort by path
        return (a.nodePath || a.nodeId).localeCompare(b.nodePath || b.nodeId)
      })

      setComparisons(nodeComparisons)
    } catch (err) {
      console.error('Failed to load comparison:', err)
      setError(err instanceof Error ? err.message : 'Failed to load comparison')
    } finally {
      setLoading(false)
    }
  }

  function toggleNodeExpanded(nodeId: string) {
    setExpandedNodes(prev => {
      const next = new Set(prev)
      if (next.has(nodeId)) {
        next.delete(nodeId)
      } else {
        next.add(nodeId)
      }
      return next
    })
  }

  function getOperationIcon(operation: string) {
    switch (operation) {
      case 'added':
        return <Plus className="w-4 h-4" />
      case 'modified':
        return <Edit className="w-4 h-4" />
      case 'deleted':
        return <Trash2 className="w-4 h-4" />
      default:
        return null
    }
  }

  function getOperationColor(operation: string) {
    switch (operation) {
      case 'added':
        return 'bg-green-500/10 border-green-500/30 text-green-400'
      case 'modified':
        return 'bg-yellow-500/10 border-yellow-500/30 text-yellow-400'
      case 'deleted':
        return 'bg-red-500/10 border-red-500/30 text-red-400'
      default:
        return 'bg-white/5 border-white/10 text-white/60'
    }
  }


  if (loading) {
    return (
      <div className="h-full flex items-center justify-center bg-black/20">
        <div className="flex items-center gap-3 text-white/60">
          <Loader2 className="w-6 h-6 animate-spin" />
          <span>Comparing revisions #{fromRevision} and #{toRevision}...</span>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-full flex items-center justify-center bg-black/20">
        <div className="text-center">
          <div className="text-red-400 mb-2">Failed to load comparison</div>
          <div className="text-white/60 text-sm">{error}</div>
          <button
            onClick={onClose}
            className="mt-4 px-4 py-2 bg-white/10 hover:bg-white/20 rounded-lg text-white transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full flex flex-col bg-black/20">
      {/* Header */}
      <div className="flex-shrink-0 bg-black/30 backdrop-blur-md border-b border-white/10 p-6">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <GitCompare className="w-6 h-6 text-purple-400" />
            <div>
              <h2 className="text-xl font-bold text-white">
                Compare Revisions
              </h2>
              <p className="text-sm text-white/60 mt-1">
                Showing changes from <span className="font-mono text-purple-400">#{fromRevision}</span> to{' '}
                <span className="font-mono text-purple-400">#{toRevision}</span>
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
            title="Close comparison"
          >
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        {/* Summary */}
        <div className="mt-4 flex items-center gap-4 text-sm flex-wrap">
          <div className="flex items-center gap-2">
            <span className="text-green-400 font-semibold">
              +{comparisons.filter(c => c.operation === 'added').length}
            </span>
            <span className="text-white/60">added</span>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-yellow-400 font-semibold">
              ~{comparisons.filter(c => c.operation === 'modified').length}
            </span>
            <span className="text-white/60">modified</span>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-red-400 font-semibold">
              -{comparisons.filter(c => c.operation === 'deleted').length}
            </span>
            <span className="text-white/60">deleted</span>
          </div>
          {comparisons.some(c => c.translationLocale) && (
            <>
              <div className="w-px h-4 bg-white/20"></div>
              <div className="flex items-center gap-2">
                <Globe className="w-4 h-4 text-accent-400" />
                <span className="text-accent-400 font-semibold">
                  {comparisons.filter(c => c.translationLocale).length}
                </span>
                <span className="text-white/60">translation{comparisons.filter(c => c.translationLocale).length !== 1 ? 's' : ''}</span>
              </div>
            </>
          )}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {comparisons.length === 0 ? (
          <div className="text-center py-12 text-white/40">
            <GitCompare className="w-16 h-16 mx-auto mb-4 opacity-30" />
            <p>No changes between these revisions</p>
          </div>
        ) : (
          <div className="space-y-3 max-w-5xl">
            {comparisons.map((comparison) => {
              const isExpanded = expandedNodes.has(comparison.nodeId)
              const hasDetails = comparison.operation === 'modified' && comparison.propertyDiffs && comparison.propertyDiffs.length > 0

              return (
                <div
                  key={comparison.nodeId}
                  className={`border rounded-lg overflow-hidden ${getOperationColor(comparison.operation)}`}
                >
                  {/* Node header */}
                  <div
                    className={`p-4 ${hasDetails ? 'cursor-pointer hover:bg-white/5' : ''}`}
                    onClick={() => hasDetails && toggleNodeExpanded(comparison.nodeId)}
                  >
                    <div className="flex items-start justify-between">
                      <div className="flex items-start gap-3 flex-1 min-w-0">
                        <div className="flex-shrink-0 mt-1">
                          {getOperationIcon(comparison.operation)}
                        </div>
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2 mb-1 flex-wrap">
                            <span className="font-semibold capitalize">
                              {comparison.operation}
                            </span>
                            {comparison.nodeType && (
                              <span className="px-2 py-0.5 rounded bg-white/10 text-xs font-mono">
                                {comparison.nodeType}
                              </span>
                            )}
                            {comparison.translationLocale && (
                              <span className="flex items-center gap-1 px-2 py-0.5 rounded bg-accent-500/20 text-accent-300 text-xs font-mono">
                                <Globe className="w-3 h-3" />
                                {comparison.translationLocale}
                              </span>
                            )}
                          </div>
                          <p className="text-sm opacity-90 truncate">
                            {comparison.nodePath || comparison.nodeId}
                          </p>
                          {comparison.translationLocale ? (
                            <p className="text-xs opacity-60 mt-1">
                              Translation change
                            </p>
                          ) : comparison.operation === 'modified' && comparison.propertyDiffs ? (
                            <p className="text-xs opacity-60 mt-1">
                              {comparison.propertyDiffs.length} {comparison.propertyDiffs.length === 1 ? 'property' : 'properties'} changed
                            </p>
                          ) : null}
                        </div>
                      </div>
                      {hasDetails && (
                        <button className="flex-shrink-0 ml-2">
                          {isExpanded ? (
                            <ChevronDown className="w-5 h-5" />
                          ) : (
                            <ChevronRight className="w-5 h-5" />
                          )}
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Property diffs (for modified nodes) */}
                  {isExpanded && hasDetails && (
                    <div className="border-t border-current/20 bg-black/20 p-4">
                      <div className="space-y-3">
                        {comparison.propertyDiffs!.map((diff, idx) => (
                          <div key={idx} className="font-mono text-xs">
                            <div className="text-white/60 mb-1">
                              {formatPropertyPath(diff.path)}
                            </div>
                            <div className="space-y-1">
                              {diff.type === 'removed' && (
                                <div className="text-red-400">
                                  - {formatDiffValue(diff.oldValue)}
                                </div>
                              )}
                              {diff.type === 'added' && (
                                <div className="text-green-400">
                                  + {formatDiffValue(diff.newValue)}
                                </div>
                              )}
                              {diff.type === 'modified' && (
                                <>
                                  <div className="text-red-400">
                                    - {formatDiffValue(diff.oldValue)}
                                  </div>
                                  <div className="text-green-400">
                                    + {formatDiffValue(diff.newValue)}
                                  </div>
                                </>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </div>
    </div>
  )
}
