import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { RefreshCw, AlertTriangle, ArrowRight } from 'lucide-react'
import { systemUpdatesApi, PendingUpdatesResponse } from '../api/system-updates'

interface SystemUpdateBannerProps {
  tenant: string
  repo: string
}

export default function SystemUpdateBanner({ tenant, repo }: SystemUpdateBannerProps) {
  const [updates, setUpdates] = useState<PendingUpdatesResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const checkUpdates = async () => {
      try {
        setLoading(true)
        setError(null)
        const response = await systemUpdatesApi.getPending(tenant, repo)
        setUpdates(response)
      } catch (err) {
        console.error('Failed to check for system updates:', err)
        setError('Failed to check for updates')
      } finally {
        setLoading(false)
      }
    }

    checkUpdates()
    // Check for updates every 5 minutes
    const interval = setInterval(checkUpdates, 5 * 60 * 1000)
    return () => clearInterval(interval)
  }, [tenant, repo])

  // Don't show banner while loading, on error, or if no updates
  if (loading || error || !updates?.has_updates) {
    return null
  }

  const hasBreaking = updates.breaking_count > 0

  return (
    <div
      className={`border-b px-6 py-3 ${
        hasBreaking
          ? 'bg-gradient-to-r from-red-600/90 to-orange-600/90 border-red-500/50'
          : 'bg-gradient-to-r from-amber-600/90 to-yellow-600/90 border-amber-500/50'
      }`}
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          {hasBreaking ? (
            <AlertTriangle className="w-5 h-5 text-white" />
          ) : (
            <RefreshCw className="w-5 h-5 text-white" />
          )}
          <div>
            <span className="text-white font-semibold">
              {updates.total_pending} system update{updates.total_pending !== 1 ? 's' : ''} available
            </span>
            {hasBreaking && (
              <span className="ml-2 text-white/90 text-sm bg-red-900/40 px-2 py-0.5 rounded">
                {updates.breaking_count} breaking
              </span>
            )}
            <span className="ml-2 text-white/80 text-sm">
              {updates.updates.filter(u => u.resource_type === 'NodeType').length} NodeTypes,{' '}
              {updates.updates.filter(u => u.resource_type === 'Workspace').length} Workspaces,{' '}
              {updates.updates.filter(u => u.resource_type === 'Package').length} Packages
            </span>
          </div>
        </div>

        <Link
          to={`/${repo}/system-updates`}
          className="flex items-center gap-2 px-4 py-2 rounded bg-white/20 hover:bg-white/30 text-white font-medium transition-colors"
        >
          Review Updates
          <ArrowRight className="w-4 h-4" />
        </Link>
      </div>
    </div>
  )
}
