import { useEffect, useState } from 'react'
import { Activity, Database, HardDrive, Zap, TrendingUp, Users } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import MetricCard from '../../components/management/MetricCard'
import { managementApi, Metrics as MetricsType, sseManager, formatBytes } from '../../api/management'

export default function Metrics() {
  const [metrics, setMetrics] = useState<MetricsType | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)

  // Fetch initial metrics
  useEffect(() => {
    const fetchMetrics = async () => {
      try {
        const response = await managementApi.getMetrics()
        if (response.success && response.data) {
          setMetrics(response.data)
        } else {
          setError(response.error || 'Failed to fetch metrics')
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch metrics')
      } finally {
        setLoading(false)
      }
    }

    fetchMetrics()
  }, [])

  // Connect to SSE for live metric updates
  useEffect(() => {
    const cleanup = sseManager.connect('metrics', {
      onMetricsUpdate: (newMetrics) => {
        setMetrics(newMetrics)
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
            {[1, 2, 3, 4, 5, 6].map((i) => (
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

  if (!metrics) {
    return <div className="pt-8 text-gray-400">No metrics available</div>
  }

  return (
    <div className="pt-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2">System Metrics</h1>
          <p className="text-gray-400">Real-time performance monitoring</p>
        </div>
        <div className="flex items-center gap-2">
          <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
          <span className="text-sm text-gray-400">{connected ? 'Live Updates' : 'Disconnected'}</span>
        </div>
      </div>

      {/* Main Metrics Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6 mb-8">
        <MetricCard
          title="Operations/Second"
          value={metrics.operations_per_sec.toFixed(1)}
          icon={Activity}
          subtitle="Current throughput"
        />
        <MetricCard
          title="Error Rate"
          value={`${(metrics.error_rate * 100).toFixed(2)}%`}
          icon={TrendingUp}
          subtitle={metrics.error_rate > 0.01 ? 'Above threshold' : 'Within limits'}
        />
        <MetricCard
          title="Disk Usage"
          value={formatBytes(metrics.disk_usage_bytes)}
          icon={HardDrive}
          subtitle="Total storage used"
        />
        <MetricCard
          title="Node Count"
          value={metrics.node_count.toLocaleString()}
          icon={Database}
          subtitle="Total nodes stored"
        />
        <MetricCard
          title="Active Connections"
          value={metrics.active_connections}
          icon={Users}
          subtitle="Currently connected"
        />
        <MetricCard
          title="Cache Hit Rate"
          value={`${(metrics.cache_hit_rate * 100).toFixed(1)}%`}
          icon={Zap}
          subtitle="Cache efficiency"
        />
      </div>

      {/* Index Sizes */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4">Index Sizes</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {Object.entries(metrics.index_sizes).map(([name, size]) => (
            <GlassCard key={name}>
              <div className="flex items-center justify-between">
                <div>
                  <p className="text-sm text-gray-400 mb-1 capitalize">{name} Index</p>
                  <p className="text-2xl font-bold text-white">{formatBytes(size)}</p>
                </div>
                <div className="p-3 bg-purple-500/20 rounded-lg">
                  <Database className="w-5 h-5 text-purple-400" />
                </div>
              </div>
            </GlassCard>
          ))}
        </div>
      </div>

      {/* Compaction Info */}
      <div>
        <h2 className="text-xl font-semibold text-white mb-4">Maintenance</h2>
        <GlassCard>
          <div className="flex items-start gap-3">
            <div className="p-3 bg-blue-500/20 rounded-lg">
              <Zap className="w-6 h-6 text-blue-400" />
            </div>
            <div className="flex-1">
              <h3 className="text-lg font-semibold text-white mb-1">Last Compaction</h3>
              <p className="text-gray-400">
                {metrics.last_compaction
                  ? new Date(metrics.last_compaction).toLocaleString()
                  : 'Never'}
              </p>
            </div>
          </div>
        </GlassCard>
      </div>
    </div>
  )
}
