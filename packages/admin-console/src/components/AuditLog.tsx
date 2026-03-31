import { useState, useEffect } from 'react'
import { FileText, Clock, User, Activity } from 'lucide-react'

interface AuditEntry {
  id: string
  node_id: string
  action: string
  user_id?: string | null
  changes?: any
  timestamp: string
}

interface AuditLogProps {
  repo: string
  branch: string
  workspace: string
  nodePath: string
}

export default function AuditLog({ repo, branch, workspace, nodePath }: AuditLogProps) {
  const [logs, setLogs] = useState<AuditEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    fetchLogs()
  }, [repo, branch, workspace, nodePath])

  async function fetchLogs() {
    try {
      setLoading(true)
      setError(null)
      const response = await fetch(`/api/repository/${repo}/${branch}/${workspace}${nodePath}/raisin:cmd/audit_log`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      })
      if (!response.ok) throw new Error('Failed to fetch audit logs')
      const data = await response.json()
      setLogs(Array.isArray(data) ? data : [])
    } catch (err: any) {
      setError(err.message)
    } finally {
      setLoading(false)
    }
  }

  function getActionIcon(action: string) {
    switch (action?.toLowerCase()) {
      case 'create':
      case 'created':
        return <FileText className="w-4 h-4 text-green-400" />
      case 'update':
      case 'updated':
        return <Activity className="w-4 h-4 text-secondary-400" />
      case 'delete':
      case 'deleted':
        return <FileText className="w-4 h-4 text-red-400" />
      case 'publish':
      case 'published':
        return <Activity className="w-4 h-4 text-primary-400" />
      default:
        return <Activity className="w-4 h-4 text-gray-400" />
    }
  }

  function getActionColor(action: string) {
    switch (action?.toLowerCase()) {
      case 'create':
      case 'created':
        return 'text-green-300'
      case 'update':
      case 'updated':
        return 'text-secondary-300'
      case 'delete':
      case 'deleted':
        return 'text-red-300'
      case 'publish':
      case 'published':
        return 'text-primary-300'
      default:
        return 'text-gray-300'
    }
  }

  function formatDate(dateStr: string) {
    const date = new Date(dateStr)
    return date.toLocaleString()
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <div className="text-gray-400">Loading audit logs...</div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-4 bg-red-500/20 border border-red-500/50 rounded-lg text-red-300">
        {error}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Clock className="w-5 h-5 text-gray-400" />
        <h3 className="text-lg font-semibold text-white">
          Audit Log ({logs.length})
        </h3>
      </div>

      {logs.length === 0 ? (
        <div className="text-center text-gray-400 py-8">
          No audit entries found
        </div>
      ) : (
        <div className="space-y-2">
          {logs.map((entry) => (
            <div
              key={entry.id}
              className="glass-dark rounded-lg p-4 hover:bg-white/5 transition-colors"
            >
              <div className="flex items-start justify-between">
                <div className="flex items-start gap-3 flex-1">
                  <div className="p-2 bg-white/5 rounded">
                    {getActionIcon(entry.action)}
                  </div>
                  <div className="flex-1">
                    <div className="flex items-center gap-2">
                      <span className={`font-semibold ${getActionColor(entry.action)}`}>
                        {entry.action}
                      </span>
                      {entry.user_id && (
                        <span className="flex items-center gap-1 text-sm text-gray-400">
                          <User className="w-3 h-3" />
                          {entry.user_id}
                        </span>
                      )}
                    </div>
                    <div className="mt-1 text-sm text-gray-400">
                      {formatDate(entry.timestamp)}
                    </div>
                    {entry.changes && (
                      <div className="mt-2 p-2 bg-black/30 rounded text-xs font-mono text-gray-300 overflow-auto max-h-40">
                        {JSON.stringify(entry.changes, null, 2)}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
