import { useEffect, useState } from 'react'
import { Clock, MoveRight, Copy, FileEdit, ArrowUpDown, User, Calendar, ExternalLink } from 'lucide-react'
import { revisionsApi, RevisionMeta } from '../api/revisions'
import { Link } from 'react-router-dom'

interface NodeOperationHistoryProps {
  repo: string
  branch: string
  workspace: string
  nodeId: string
  nodePath: string
  limit?: number
}

export default function NodeOperationHistory({
  repo,
  branch,
  workspace,
  nodeId,
  limit = 20
}: NodeOperationHistoryProps) {
  const [revisions, setRevisions] = useState<RevisionMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadRevisions()
  }, [repo, branch, nodeId])

  async function loadRevisions() {
    try {
      setLoading(true)
      setError(null)

      // Fetch all revisions for the repository
      const allRevisions = await revisionsApi.list(repo, limit, 0, true, branch)

      // Filter to only revisions that affected this node
      // TODO: Backend should provide a node-specific endpoint
      // For now, we'll fetch revision details and filter client-side
      const nodeRevisions: RevisionMeta[] = []

      for (const rev of allRevisions) {
        try {
          const revisionDetail = await revisionsApi.get(repo, rev.number)
          // Check if this revision has an operation on our node
          if (revisionDetail.operation?.node_id === nodeId) {
            nodeRevisions.push(revisionDetail)
          }
        } catch (err) {
          console.warn(`Failed to fetch revision ${rev.number}:`, err)
        }
      }

      setRevisions(nodeRevisions)
    } catch (err) {
      console.error('Failed to load revisions:', err)
      setError('Failed to load operation history')
    } finally {
      setLoading(false)
    }
  }

  function getOperationIcon(type?: string) {
    switch (type) {
      case 'move':
        return <MoveRight className="w-5 h-5 text-blue-400" />
      case 'copy':
        return <Copy className="w-5 h-5 text-green-400" />
      case 'rename':
        return <FileEdit className="w-5 h-5 text-yellow-400" />
      case 'reorder':
        return <ArrowUpDown className="w-5 h-5 text-purple-400" />
      default:
        return <Clock className="w-5 h-5 text-zinc-400" />
    }
  }

  function getOperationDescription(revision: RevisionMeta) {
    const op = revision.operation
    if (!op) {
      return <span className="text-zinc-400">No operation details</span>
    }

    const opType = op.operation

    switch (opType.type) {
      case 'move':
        return (
          <div className="space-y-1">
            <div className="text-zinc-200">
              Moved node
            </div>
            <div className="text-sm text-zinc-400 space-y-0.5">
              <div>From: <code className="text-xs bg-zinc-800 px-1.5 py-0.5 rounded">{opType.from_path}</code></div>
              <div>To: <code className="text-xs bg-zinc-800 px-1.5 py-0.5 rounded">{opType.to_path}</code></div>
            </div>
          </div>
        )

      case 'copy':
        return (
          <div className="space-y-1">
            <div className="text-zinc-200">
              Copied from another node
            </div>
            <div className="text-sm text-zinc-400 space-y-0.5">
              <div>Source: <code className="text-xs bg-zinc-800 px-1.5 py-0.5 rounded">{opType.source_path}</code></div>
              <div>Destination: <code className="text-xs bg-zinc-800 px-1.5 py-0.5 rounded">{opType.destination_path}</code></div>
            </div>
          </div>
        )

      case 'rename':
        return (
          <div className="space-y-1">
            <div className="text-zinc-200">
              Renamed node
            </div>
            <div className="text-sm text-zinc-400">
              <span className="line-through">{opType.old_name}</span> → {opType.new_name}
            </div>
          </div>
        )

      case 'reorder':
        return (
          <div className="text-zinc-200">
            Reordered within parent
          </div>
        )

      default:
        return <span className="text-zinc-400">Unknown operation</span>
    }
  }

  function formatTimestamp(timestamp: string) {
    const date = new Date(timestamp)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffMins = Math.floor(diffMs / 60000)
    const diffHours = Math.floor(diffMs / 3600000)
    const diffDays = Math.floor(diffMs / 86400000)

    if (diffMins < 1) return 'Just now'
    if (diffMins < 60) return `${diffMins}m ago`
    if (diffHours < 24) return `${diffHours}h ago`
    if (diffDays < 7) return `${diffDays}d ago`

    return date.toLocaleDateString(undefined, {
      month: 'short',
      day: 'numeric',
      year: date.getFullYear() !== now.getFullYear() ? 'numeric' : undefined
    })
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary-500"></div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="text-center py-12">
        <p className="text-red-400">{error}</p>
        <button
          onClick={loadRevisions}
          className="mt-4 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          Retry
        </button>
      </div>
    )
  }

  if (revisions.length === 0) {
    return (
      <div className="text-center py-12 text-zinc-400">
        <Clock className="w-12 h-12 mx-auto mb-2 opacity-50" />
        <p>No operation history available</p>
        <p className="text-sm mt-2">Operations performed on this node will appear here</p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between mb-4">
        <h3 className="text-lg font-semibold text-zinc-200">Operation History</h3>
        <span className="text-sm text-zinc-400">{revisions.length} operations</span>
      </div>

      <div className="space-y-4">
        {revisions.map((revision) => (
          <div
            key={revision.revision}
            className="relative pl-8 pb-6 border-l-2 border-zinc-700 last:border-l-0 last:pb-0"
          >
            {/* Timeline dot */}
            <div className="absolute left-0 top-0 -translate-x-1/2 bg-zinc-900 p-1 rounded-full border-2 border-zinc-700">
              {getOperationIcon(revision.operation?.operation.type)}
            </div>

            {/* Content */}
            <div className="bg-zinc-800/50 rounded-lg p-4 space-y-3">
              {/* Header */}
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  {getOperationDescription(revision)}
                </div>
                <Link
                  to={`/${repo}/content/${branch}/${workspace}?rev=${revision.revision}`}
                  className="flex-shrink-0 text-primary-400 hover:text-primary-300 transition-colors"
                  title="View at this revision"
                >
                  <ExternalLink className="w-4 h-4" />
                </Link>
              </div>

              {/* Message */}
              {revision.message && (
                <div className="text-sm text-zinc-300 italic border-l-2 border-zinc-600 pl-3">
                  "{revision.message}"
                </div>
              )}

              {/* Metadata */}
              <div className="flex items-center gap-4 text-xs text-zinc-400">
                <div className="flex items-center gap-1">
                  <User className="w-3 h-3" />
                  {revision.actor}
                </div>
                <div className="flex items-center gap-1">
                  <Calendar className="w-3 h-3" />
                  {formatTimestamp(revision.timestamp)}
                </div>
                <div className="flex items-center gap-1">
                  <Clock className="w-3 h-3" />
                  Rev {revision.revision}
                </div>
                {revision.is_system && (
                  <span className="px-2 py-0.5 bg-zinc-700 rounded text-xs">
                    System
                  </span>
                )}
              </div>
            </div>
          </div>
        ))}
      </div>

      {revisions.length >= limit && (
        <div className="text-center pt-4">
          <p className="text-sm text-zinc-400">
            Showing last {limit} operations
          </p>
        </div>
      )}
    </div>
  )
}
