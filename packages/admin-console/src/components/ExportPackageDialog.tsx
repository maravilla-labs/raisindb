import { useState, useEffect } from 'react'
import {
  Download,
  FileArchive,
  Filter,
  Loader2,
  CheckCircle2,
  XCircle,
  AlertTriangle,
} from 'lucide-react'
import { packagesApi, type ExportOptions } from '../api/packages'
import { jobsApi, type JobEventData } from '../api/jobs'

interface ExportPackageDialogProps {
  open: boolean
  repo: string
  packageName: string
  branch?: string
  onClose: () => void
  onSuccess?: (jobId: string) => void
}

type ExportMode = 'all' | 'filtered'
type ExportStatus = 'idle' | 'exporting' | 'completed' | 'failed'

export default function ExportPackageDialog({
  open,
  repo,
  packageName,
  branch = 'main',
  onClose,
  onSuccess,
}: ExportPackageDialogProps) {
  const [exportMode, setExportMode] = useState<ExportMode>('all')
  const [includeModifications, setIncludeModifications] = useState(true)
  const [filterPatterns, setFilterPatterns] = useState('')
  const [status, setStatus] = useState<ExportStatus>('idle')
  const [progress, setProgress] = useState<number | null>(null)
  const [jobId, setJobId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Reset state when dialog opens/closes
  useEffect(() => {
    if (open) {
      setStatus('idle')
      setProgress(null)
      setJobId(null)
      setError(null)
    }
  }, [open])

  // Subscribe to job events when exporting
  useEffect(() => {
    if (!jobId || status !== 'exporting') return

    const cleanup = jobsApi.subscribeToJobEvents((event: JobEventData) => {
      if (event.job_id !== jobId) return

      if (event.progress !== undefined) {
        setProgress(Math.round(event.progress * 100))
      }

      if (event.status === 'Completed') {
        setStatus('completed')
        setProgress(100)
        onSuccess?.(jobId)
      } else if (event.status.startsWith('Failed')) {
        setStatus('failed')
        setError(event.error || event.status.replace('Failed: ', ''))
      }
    })

    return cleanup
  }, [jobId, status, onSuccess])

  async function handleExport() {
    setStatus('exporting')
    setProgress(0)
    setError(null)

    const options: ExportOptions = {
      export_mode: exportMode,
      include_modifications: includeModifications,
      filter_patterns: exportMode === 'filtered' && filterPatterns
        ? filterPatterns.split('\n').map(p => p.trim()).filter(p => p)
        : undefined,
    }

    try {
      const response = await packagesApi.exportPackage(repo, packageName, options, branch)
      setJobId(response.job_id)
    } catch (err) {
      setStatus('failed')
      setError(err instanceof Error ? err.message : 'Failed to start export')
    }
  }

  function handleDownload() {
    if (!jobId) return
    const url = packagesApi.getExportDownloadUrl(repo, packageName, jobId, branch)
    window.open(url, '_blank')
  }

  if (!open) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={status === 'idle' ? onClose : undefined}
      />

      {/* Dialog */}
      <div className="relative w-full max-w-lg bg-zinc-900 border border-white/10 rounded-xl shadow-2xl">
        {/* Header */}
        <div className="flex items-center gap-3 px-6 py-4 border-b border-white/10">
          <FileArchive className="w-6 h-6 text-primary-400" />
          <h2 className="text-xl font-semibold text-white">Export Package</h2>
        </div>

        {/* Content */}
        <div className="px-6 py-4 space-y-4">
          {status === 'idle' && (
            <>
              <p className="text-zinc-400 text-sm">
                Export <span className="text-white font-medium">{packageName}</span> as a .rap package file.
              </p>

              {/* Export Mode */}
              <div className="space-y-2">
                <label className="block text-sm font-medium text-zinc-300">Export Mode</label>
                <div className="flex gap-3">
                  <button
                    type="button"
                    onClick={() => setExportMode('all')}
                    className={`flex-1 flex items-center gap-2 px-4 py-3 rounded-lg border transition-colors ${
                      exportMode === 'all'
                        ? 'border-primary-500 bg-primary-500/20 text-primary-400'
                        : 'border-white/10 bg-white/5 text-zinc-400 hover:bg-white/10'
                    }`}
                  >
                    <Download className="w-5 h-5" />
                    <div className="text-left">
                      <div className="font-medium">All Files</div>
                      <div className="text-xs opacity-70">Export entire package</div>
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => setExportMode('filtered')}
                    className={`flex-1 flex items-center gap-2 px-4 py-3 rounded-lg border transition-colors ${
                      exportMode === 'filtered'
                        ? 'border-primary-500 bg-primary-500/20 text-primary-400'
                        : 'border-white/10 bg-white/5 text-zinc-400 hover:bg-white/10'
                    }`}
                  >
                    <Filter className="w-5 h-5" />
                    <div className="text-left">
                      <div className="font-medium">Filtered</div>
                      <div className="text-xs opacity-70">Use custom patterns</div>
                    </div>
                  </button>
                </div>
              </div>

              {/* Filter Patterns (when filtered mode) */}
              {exportMode === 'filtered' && (
                <div className="space-y-2">
                  <label className="block text-sm font-medium text-zinc-300">
                    Filter Patterns (one per line)
                  </label>
                  <textarea
                    value={filterPatterns}
                    onChange={(e) => setFilterPatterns(e.target.value)}
                    placeholder={`**/content/**\n!**/drafts/**\n*.yaml`}
                    className="w-full h-24 bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-white text-sm font-mono placeholder:text-zinc-600 focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                  />
                  <p className="text-xs text-zinc-500">
                    Use glob patterns. Prefix with ! to exclude.
                  </p>
                </div>
              )}

              {/* Include Modifications */}
              <label className="flex items-center gap-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={includeModifications}
                  onChange={(e) => setIncludeModifications(e.target.checked)}
                  className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-primary-500/50"
                />
                <span className="text-sm text-zinc-300">
                  Include modifications (export current state)
                </span>
              </label>
            </>
          )}

          {status === 'exporting' && (
            <div className="py-8 text-center">
              <Loader2 className="w-12 h-12 text-primary-400 animate-spin mx-auto mb-4" />
              <p className="text-white font-medium mb-2">Exporting package...</p>
              {progress !== null && (
                <div className="w-full max-w-xs mx-auto">
                  <div className="h-2 bg-white/10 rounded-full overflow-hidden">
                    <div
                      className="h-full bg-primary-500 transition-all duration-300"
                      style={{ width: `${progress}%` }}
                    />
                  </div>
                  <p className="text-sm text-zinc-400 mt-2">{progress}%</p>
                </div>
              )}
            </div>
          )}

          {status === 'completed' && (
            <div className="py-8 text-center">
              <CheckCircle2 className="w-12 h-12 text-green-400 mx-auto mb-4" />
              <p className="text-white font-medium mb-4">Export completed!</p>
              <button
                onClick={handleDownload}
                className="inline-flex items-center gap-2 px-6 py-3 bg-green-500/20 hover:bg-green-500/30 text-green-400 rounded-lg transition-colors"
              >
                <Download className="w-5 h-5" />
                Download {packageName}.rap
              </button>
            </div>
          )}

          {status === 'failed' && (
            <div className="py-8 text-center">
              <XCircle className="w-12 h-12 text-red-400 mx-auto mb-4" />
              <p className="text-white font-medium mb-2">Export failed</p>
              {error && (
                <div className="flex items-start gap-2 p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-left">
                  <AlertTriangle className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" />
                  <p className="text-sm text-red-300">{error}</p>
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-3 px-6 py-4 border-t border-white/10">
          {status === 'idle' && (
            <>
              <button
                onClick={onClose}
                className="px-4 py-2 text-zinc-400 hover:text-white transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleExport}
                className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
              >
                <FileArchive className="w-4 h-4" />
                Export Package
              </button>
            </>
          )}
          {status === 'exporting' && (
            <button
              disabled
              className="px-4 py-2 text-zinc-500 cursor-not-allowed"
            >
              Please wait...
            </button>
          )}
          {(status === 'completed' || status === 'failed') && (
            <button
              onClick={onClose}
              className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
            >
              Close
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
