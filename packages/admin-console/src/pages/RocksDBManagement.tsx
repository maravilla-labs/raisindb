import { useState, useEffect } from 'react'
import { Database, Zap, BarChart3, Settings, AlertCircle, RefreshCw, XCircle, CheckCircle } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { managementApi, formatBytes, formatDuration, sseManager, JobEvent } from '../api/management'

export default function RocksDBManagement() {
  const [tenant] = useState('default')

  // Compaction state
  const [compactLoading, setCompactLoading] = useState(false)
  const [compactResult, setCompactResult] = useState<any>(null)
  const [compactJobId, setCompactJobId] = useState<string | null>(null)
  const [compactProgress, setCompactProgress] = useState<number>(0)

  // Connect to SSE for compaction job updates
  useEffect(() => {
    if (!compactJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        // Only process events for our job
        if (event.job_id !== compactJobId) return

        // Update progress
        if (event.progress !== null && event.progress !== undefined) {
          setCompactProgress(event.progress)
        }

        // Handle completion
        if (event.status === 'Completed') {
          managementApi.getJobInfo(compactJobId).then(response => {
            if (response.success && response.data?.result) {
              setCompactResult({ type: 'success', data: response.data.result })
            }
            setCompactLoading(false)
            setCompactJobId(null)
            setCompactProgress(0)
          })
        }

        // Handle failure
        if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
          setCompactResult({ type: 'error', message: event.error || 'Compaction failed' })
          setCompactLoading(false)
          setCompactJobId(null)
          setCompactProgress(0)
        }
      },
      onError: () => {
        console.error('SSE connection error for compaction job')
      }
    })

    return cleanup
  }, [compactJobId])

  const handleTriggerCompaction = async () => {
    setCompactLoading(true)
    setCompactResult(null)
    setCompactProgress(0)

    try {
      // Start the background job
      const response = await managementApi.startCompaction()
      if (response.success && response.data) {
        setCompactJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setCompactResult({ type: 'error', message: response.error || 'Failed to start compaction' })
        setCompactLoading(false)
      }
    } catch (error) {
      setCompactResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setCompactLoading(false)
    }
  }

  const handleCancelCompaction = async () => {
    if (!compactJobId) return

    try {
      await managementApi.cancelJob(compactJobId)
      setCompactLoading(false)
      setCompactJobId(null)
      setCompactProgress(0)
      setCompactResult({ type: 'warning', message: 'Compaction cancelled' })
    } catch (error) {
      console.error('Failed to cancel compaction:', error)
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold text-white mb-2">RocksDB Operations</h1>
        <p className="text-white/60">
          Manage RocksDB storage engine settings and operations (Tenant: {tenant})
        </p>
      </div>

      {/* Info Banner */}
      <div className="flex items-start gap-3 p-4 bg-blue-500/10 border border-blue-400/30 rounded-lg">
        <AlertCircle className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
        <div className="text-sm text-blue-200">
          <p className="font-semibold mb-1">Local Development Mode</p>
          <p className="text-blue-300/80">
            RocksDB operations are enabled for the default tenant in local development environments.
            These operations provide direct access to the underlying storage engine.
          </p>
        </div>
      </div>

      {/* Manual Compaction - Active */}
      <GlassCard>
        <div className="flex items-start gap-4 mb-4">
          <div className="p-3 bg-primary-500/20 rounded-lg border border-primary-400/30">
            <Zap className="w-6 h-6 text-primary-400" />
          </div>
          <div className="flex-1">
            <h3 className="text-lg font-semibold text-white mb-2">Manual Compaction</h3>
            <p className="text-white/60 text-sm mb-4">
              Trigger manual compaction to optimize disk usage and improve read performance.
              This operation runs in the background and may take several minutes.
            </p>

            <div className="flex gap-2">
              <button
                onClick={handleTriggerCompaction}
                disabled={compactLoading}
                className="px-4 py-2 bg-primary-500/20 hover:bg-primary-500/30 border border-primary-500/30 rounded-lg text-primary-300 hover:text-primary-200 flex items-center gap-2 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {compactLoading ? (
                  <>
                    <RefreshCw className="w-4 h-4 animate-spin" />
                    Compacting...
                  </>
                ) : (
                  <>
                    <Zap className="w-4 h-4" />
                    Trigger Compaction
                  </>
                )}
              </button>

              {compactLoading && compactJobId && (
                <button
                  onClick={handleCancelCompaction}
                  className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
                >
                  <XCircle className="w-4 h-4" />
                  Cancel
                </button>
              )}
            </div>

            {/* Progress Bar */}
            {compactLoading && compactProgress > 0 && (
              <div className="mt-4">
                <div className="flex items-center justify-between text-sm mb-2">
                  <div className="flex items-center gap-2 text-zinc-300">
                    <RefreshCw className="w-4 h-4 animate-spin" />
                    <span>Compacting database...</span>
                  </div>
                  <span className="text-white font-medium">{Math.round(compactProgress * 100)}%</span>
                </div>
                <div className="w-full bg-white/10 rounded-full h-2">
                  <div
                    className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                    style={{ width: `${Math.round(compactProgress * 100)}%` }}
                  ></div>
                </div>
              </div>
            )}

            {/* Result Display */}
            {compactResult && (
              <div className="mt-4">
                {compactResult.type === 'success' ? (
                  <div className="p-4 bg-green-500/10 border border-green-500/30 rounded-lg">
                    <div className="flex items-start gap-3">
                      <CheckCircle className="w-5 h-5 text-green-400 flex-shrink-0 mt-0.5" />
                      <div className="flex-1">
                        <h4 className="text-green-300 font-semibold mb-2">Compaction Completed</h4>
                        <div className="text-sm text-green-300/80 space-y-1">
                          <p>Before: {formatBytes(compactResult.data.bytes_before)}</p>
                          <p>After: {formatBytes(compactResult.data.bytes_after)}</p>
                          <p>Saved: {formatBytes(compactResult.data.bytes_before - compactResult.data.bytes_after)} ({((1 - compactResult.data.bytes_after / compactResult.data.bytes_before) * 100).toFixed(1)}%)</p>
                          <p>Duration: {formatDuration(compactResult.data.duration_ms)}</p>
                          <p>Files compacted: {compactResult.data.files_compacted}</p>
                        </div>
                        <button
                          onClick={() => setCompactResult(null)}
                          className="mt-3 text-xs text-green-400 hover:text-green-300 underline"
                        >
                          Dismiss
                        </button>
                      </div>
                    </div>
                  </div>
                ) : compactResult.type === 'error' ? (
                  <div className="p-4 bg-red-500/10 border border-red-500/30 rounded-lg">
                    <div className="flex items-start gap-3">
                      <XCircle className="w-5 h-5 text-red-400 flex-shrink-0 mt-0.5" />
                      <div className="flex-1">
                        <h4 className="text-red-300 font-semibold mb-1">Compaction Failed</h4>
                        <p className="text-sm text-red-300/80">{compactResult.message}</p>
                        <button
                          onClick={() => setCompactResult(null)}
                          className="mt-3 text-xs text-red-400 hover:text-red-300 underline"
                        >
                          Dismiss
                        </button>
                      </div>
                    </div>
                  </div>
                ) : (
                  <div className="p-4 bg-amber-500/10 border border-amber-500/30 rounded-lg">
                    <div className="flex items-start gap-3">
                      <AlertCircle className="w-5 h-5 text-amber-400 flex-shrink-0 mt-0.5" />
                      <div className="flex-1">
                        <p className="text-sm text-amber-300/80">{compactResult.message}</p>
                        <button
                          onClick={() => setCompactResult(null)}
                          className="mt-3 text-xs text-amber-400 hover:text-amber-300 underline"
                        >
                          Dismiss
                        </button>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </GlassCard>

      {/* Coming Soon Operations */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {/* Statistics */}
        <GlassCard>
          <div className="flex items-start gap-4">
            <div className="p-3 bg-amber-500/20 rounded-lg border border-amber-400/30">
              <BarChart3 className="w-6 h-6 text-amber-400" />
            </div>
            <div className="flex-1">
              <h3 className="text-lg font-semibold text-white mb-2">Storage Statistics</h3>
              <p className="text-white/60 text-sm mb-4">
                View detailed statistics about RocksDB storage usage and performance metrics.
              </p>
              <button
                disabled
                className="px-4 py-2 bg-white/5 text-white/40 rounded-lg border border-white/10 cursor-not-allowed text-sm"
              >
                Coming Soon
              </button>
            </div>
          </div>
        </GlassCard>

        {/* Column Families */}
        <GlassCard>
          <div className="flex items-start gap-4">
            <div className="p-3 bg-purple-500/20 rounded-lg border border-purple-400/30">
              <Database className="w-6 h-6 text-purple-400" />
            </div>
            <div className="flex-1">
              <h3 className="text-lg font-semibold text-white mb-2">Column Families</h3>
              <p className="text-white/60 text-sm mb-4">
                Manage RocksDB column families and view their individual statistics.
              </p>
              <button
                disabled
                className="px-4 py-2 bg-white/5 text-white/40 rounded-lg border border-white/10 cursor-not-allowed text-sm"
              >
                Coming Soon
              </button>
            </div>
          </div>
        </GlassCard>

        {/* Configuration */}
        <GlassCard>
          <div className="flex items-start gap-4">
            <div className="p-3 bg-emerald-500/20 rounded-lg border border-emerald-400/30">
              <Settings className="w-6 h-6 text-emerald-400" />
            </div>
            <div className="flex-1">
              <h3 className="text-lg font-semibold text-white mb-2">Engine Configuration</h3>
              <p className="text-white/60 text-sm mb-4">
                View and modify RocksDB engine configuration parameters.
              </p>
              <button
                disabled
                className="px-4 py-2 bg-white/5 text-white/40 rounded-lg border border-white/10 cursor-not-allowed text-sm"
              >
                Coming Soon
              </button>
            </div>
          </div>
        </GlassCard>
      </div>

      {/* Planned Features */}
      <GlassCard>
        <h3 className="text-lg font-semibold text-white mb-4">Planned Features</h3>
        <div className="space-y-2 text-sm text-white/60">
          <div className="flex items-center gap-2">
            <div className="w-1.5 h-1.5 bg-primary-400 rounded-full"></div>
            <span>Real-time performance metrics and graphs</span>
          </div>
          <div className="flex items-center gap-2">
            <div className="w-1.5 h-1.5 bg-primary-400 rounded-full"></div>
            <span>Block cache statistics and management</span>
          </div>
          <div className="flex items-center gap-2">
            <div className="w-1.5 h-1.5 bg-primary-400 rounded-full"></div>
            <span>WAL (Write-Ahead Log) inspection and management</span>
          </div>
          <div className="flex items-center gap-2">
            <div className="w-1.5 h-1.5 bg-primary-400 rounded-full"></div>
            <span>SST file browser and analysis tools</span>
          </div>
          <div className="flex items-center gap-2">
            <div className="w-1.5 h-1.5 bg-primary-400 rounded-full"></div>
            <span>Backup and restore operations</span>
          </div>
        </div>
      </GlassCard>
    </div>
  )
}
