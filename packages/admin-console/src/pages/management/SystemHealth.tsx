import { useEffect, useState } from 'react'
import { Activity, Database, Zap } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import HealthIndicator from '../../components/management/HealthIndicator'
import MetricCard from '../../components/management/MetricCard'
import { managementApi, HealthStatus, sseManager } from '../../api/management'

export default function SystemHealth() {
  const [health, setHealth] = useState<HealthStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)

  // Fetch initial health data
  useEffect(() => {
    const fetchHealth = async () => {
      try {
        const response = await managementApi.getHealth()
        if (response.success && response.data) {
          setHealth(response.data)
        } else {
          setError(response.error || 'Failed to fetch health status')
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch health status')
      } finally {
        setLoading(false)
      }
    }

    fetchHealth()
  }, [])

  // Connect to SSE for live updates
  useEffect(() => {
    const cleanup = sseManager.connect('health', {
      onHealthUpdate: (newHealth) => {
        setHealth(newHealth)
        setConnected(true)
      },
      onOpen: () => {
        setConnected(true)
      },
      onError: () => {
        setConnected(false)
      },
    })

    return cleanup
  }, [])

  if (loading) {
    return (
      <div className="pt-8">
        <div className="animate-pulse">
          <div className="h-8 bg-white/10 rounded w-48 mb-8"></div>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
            {[1, 2, 3].map((i) => (
              <div key={i} className="h-32 bg-white/5 rounded-xl"></div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="pt-8">
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 text-red-300">
          {error}
        </div>
      </div>
    )
  }

  if (!health) {
    return <div className="pt-8 text-gray-400">No health data available</div>
  }

  return (
    <div className="pt-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2">System Health</h1>
          <p className="text-gray-400">Real-time system health monitoring</p>
        </div>
        <div className="flex items-center gap-3">
          <HealthIndicator status={health.status} />
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
            <span className="text-sm text-gray-400">{connected ? 'Live' : 'Disconnected'}</span>
          </div>
        </div>
      </div>

      {/* Health Checks */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4">Health Checks</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {health.checks.map((check, index) => (
            <GlassCard key={index}>
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="text-sm font-medium text-white mb-1">{check.name}</h3>
                  {check.message && <p className="text-xs text-gray-400">{check.message}</p>}
                </div>
                <HealthIndicator status={check.status} showLabel={false} />
              </div>
            </GlassCard>
          ))}
        </div>
      </div>

      {/* System Info */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        <MetricCard
          title="System Status"
          value={health.status}
          icon={Activity}
          subtitle={`Last checked: ${new Date(health.last_check).toLocaleTimeString()}`}
        />
        <MetricCard
          title="Health Checks"
          value={health.checks.length}
          icon={Database}
          subtitle={`${health.checks.filter(c => c.status === 'Healthy').length} passing`}
        />
        <MetricCard
          title="Auto-Healing"
          value={health.needs_healing ? 'Needed' : 'Not Needed'}
          icon={Zap}
          subtitle={health.needs_healing ? 'Healing required' : 'System healthy'}
        />
      </div>

      {/* Healing Alert */}
      {health.needs_healing && (
        <div className="mt-6">
          <GlassCard className="border-2 border-yellow-500/30">
            <div className="flex items-start gap-3">
              <div className="p-2 bg-yellow-500/20 rounded-lg">
                <Zap className="w-5 h-5 text-yellow-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-lg font-semibold text-white mb-1">Auto-Healing Required</h3>
                <p className="text-gray-400 text-sm">
                  The system has detected issues that require healing. Background jobs will automatically
                  attempt to resolve these issues.
                </p>
              </div>
            </div>
          </GlassCard>
        </div>
      )}
    </div>
  )
}
