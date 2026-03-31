import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import {
  ArrowLeft,
  RefreshCw,
  CheckCircle,
  AlertCircle,
  FileText,
  CloudUpload,
  CloudDownload,
  AlertTriangle,
  Eye,
  Package,
  Loader2,
} from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import DiffViewer from '../../components/DiffViewer'
import { useToast, ToastContainer } from '../../components/Toast'
import {
  packagesApi,
  type SyncStatusResponse,
  type SyncFileInfo,
  type FileDiff,
  type SyncFileStatus,
} from '../../api/packages'

type StatusFilter = 'all' | SyncFileStatus

function getStatusIcon(status: SyncFileStatus) {
  switch (status) {
    case 'synced':
      return <CheckCircle className="w-4 h-4 text-green-400" />
    case 'modified':
      return <AlertCircle className="w-4 h-4 text-yellow-400" />
    case 'local_only':
      return <CloudUpload className="w-4 h-4 text-blue-400" />
    case 'server_only':
      return <CloudDownload className="w-4 h-4 text-purple-400" />
    case 'conflict':
      return <AlertTriangle className="w-4 h-4 text-red-400" />
    default:
      return <FileText className="w-4 h-4 text-zinc-400" />
  }
}

function getStatusLabel(status: SyncFileStatus): string {
  switch (status) {
    case 'synced':
      return 'Synced'
    case 'modified':
      return 'Modified'
    case 'local_only':
      return 'Local Only'
    case 'server_only':
      return 'Server Only'
    case 'conflict':
      return 'Conflict'
    default:
      return status
  }
}

function getStatusBadgeClass(status: SyncFileStatus): string {
  switch (status) {
    case 'synced':
      return 'bg-green-500/20 text-green-400'
    case 'modified':
      return 'bg-yellow-500/20 text-yellow-400'
    case 'local_only':
      return 'bg-blue-500/20 text-blue-400'
    case 'server_only':
      return 'bg-purple-500/20 text-purple-400'
    case 'conflict':
      return 'bg-red-500/20 text-red-400'
    default:
      return 'bg-zinc-500/20 text-zinc-400'
  }
}

export default function PackageSync() {
  const navigate = useNavigate()
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const packageName = pathParam
  const currentBranch = branch || 'main'

  const [syncStatus, setSyncStatus] = useState<SyncStatusResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all')
  const [selectedFile, setSelectedFile] = useState<SyncFileInfo | null>(null)
  const [diffLoading, setDiffLoading] = useState(false)
  const [fileDiff, setFileDiff] = useState<FileDiff | null>(null)
  const { toasts, error: showError, closeToast } = useToast()

  useEffect(() => {
    loadSyncStatus()
  }, [repo, packageName, currentBranch])

  async function loadSyncStatus() {
    if (!repo || !packageName) return
    setLoading(true)
    try {
      const status = await packagesApi.getSyncStatus(repo, packageName, true, currentBranch)
      setSyncStatus(status)
    } catch (err) {
      console.error('Failed to load sync status:', err)
      showError('Load Failed', 'Failed to load sync status')
    } finally {
      setLoading(false)
    }
  }

  async function loadFileDiff(file: SyncFileInfo) {
    if (!repo || !packageName) return
    setSelectedFile(file)
    setDiffLoading(true)
    setFileDiff(null)

    try {
      const diff = await packagesApi.getFileDiff(repo, packageName, file.path, currentBranch)
      setFileDiff(diff)
    } catch (err) {
      console.error('Failed to load diff:', err)
      showError('Load Failed', 'Failed to load file diff')
    } finally {
      setDiffLoading(false)
    }
  }

  const filteredFiles = syncStatus?.files.filter(
    (f) => statusFilter === 'all' || f.status === statusFilter
  ) || []

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="flex items-center justify-center py-12">
          <Loader2 className="w-8 h-8 text-primary-400 animate-spin" />
          <span className="ml-3 text-zinc-400">Loading sync status...</span>
        </div>
      </div>
    )
  }

  if (!syncStatus) {
    return (
      <div className="animate-fade-in">
        <GlassCard>
          <div className="text-center py-12">
            <Package className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">Package not found</h3>
            <p className="text-zinc-400">Unable to load sync status for this package</p>
          </div>
        </GlassCard>
      </div>
    )
  }

  return (
    <div className="animate-fade-in">
      {/* Header */}
      <div className="mb-8">
        <button
          onClick={() => navigate(`/${repo}/${currentBranch}/packages/${encodeURIComponent(packageName || '')}`)}
          className="flex items-center gap-2 text-zinc-400 hover:text-white mb-4 transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Package
        </button>

        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-3xl font-bold text-white mb-2">Sync Status</h1>
            <p className="text-zinc-400">{syncStatus.package_name}</p>
          </div>
          <button
            onClick={loadSyncStatus}
            className="flex items-center gap-2 px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            <RefreshCw className="w-4 h-4" />
            Refresh
          </button>
        </div>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4 mb-8">
        <SummaryCard
          label="Total Files"
          value={syncStatus.summary.total_files}
          icon={<FileText className="w-5 h-5" />}
          active={statusFilter === 'all'}
          onClick={() => setStatusFilter('all')}
        />
        <SummaryCard
          label="Synced"
          value={syncStatus.summary.synced}
          icon={<CheckCircle className="w-5 h-5" />}
          color="green"
          active={statusFilter === 'synced'}
          onClick={() => setStatusFilter('synced')}
        />
        <SummaryCard
          label="Modified"
          value={syncStatus.summary.modified}
          icon={<AlertCircle className="w-5 h-5" />}
          color="yellow"
          active={statusFilter === 'modified'}
          onClick={() => setStatusFilter('modified')}
        />
        <SummaryCard
          label="Local Only"
          value={syncStatus.summary.local_only}
          icon={<CloudUpload className="w-5 h-5" />}
          color="blue"
          active={statusFilter === 'local_only'}
          onClick={() => setStatusFilter('local_only')}
        />
        <SummaryCard
          label="Server Only"
          value={syncStatus.summary.server_only}
          icon={<CloudDownload className="w-5 h-5" />}
          color="purple"
          active={statusFilter === 'server_only'}
          onClick={() => setStatusFilter('server_only')}
        />
        <SummaryCard
          label="Conflicts"
          value={syncStatus.summary.conflicts}
          icon={<AlertTriangle className="w-5 h-5" />}
          color="red"
          active={statusFilter === 'conflict'}
          onClick={() => setStatusFilter('conflict')}
        />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* File List */}
        <GlassCard>
          <h2 className="text-lg font-semibold text-white mb-4">
            Files
            {statusFilter !== 'all' && (
              <span className="ml-2 text-sm font-normal text-zinc-400">
                ({getStatusLabel(statusFilter as SyncFileStatus)})
              </span>
            )}
          </h2>

          {filteredFiles.length === 0 ? (
            <div className="text-center py-8 text-zinc-500">
              No files match the selected filter
            </div>
          ) : (
            <div className="space-y-1 max-h-[600px] overflow-y-auto">
              {filteredFiles.map((file) => (
                <button
                  key={file.path}
                  onClick={() => loadFileDiff(file)}
                  className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg text-left transition-colors ${
                    selectedFile?.path === file.path
                      ? 'bg-primary-500/20 border border-primary-500/30'
                      : 'hover:bg-white/5'
                  }`}
                >
                  {getStatusIcon(file.status)}
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-white truncate">{file.path}</p>
                    {file.modified_at && (
                      <p className="text-xs text-zinc-500">
                        Modified {new Date(file.modified_at).toLocaleDateString()}
                      </p>
                    )}
                  </div>
                  <span className={`px-2 py-0.5 text-xs rounded ${getStatusBadgeClass(file.status)}`}>
                    {getStatusLabel(file.status)}
                  </span>
                  {file.status !== 'synced' && (
                    <Eye className="w-4 h-4 text-zinc-500" />
                  )}
                </button>
              ))}
            </div>
          )}
        </GlassCard>

        {/* Diff Viewer */}
        <div>
          {selectedFile ? (
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <h2 className="text-lg font-semibold text-white">
                  Diff: {selectedFile.path}
                </h2>
                <span className={`px-2 py-1 text-xs rounded ${getStatusBadgeClass(selectedFile.status)}`}>
                  {getStatusLabel(selectedFile.status)}
                </span>
              </div>

              {diffLoading ? (
                <GlassCard>
                  <div className="flex items-center justify-center py-12">
                    <Loader2 className="w-6 h-6 text-primary-400 animate-spin" />
                    <span className="ml-2 text-zinc-400">Loading diff...</span>
                  </div>
                </GlassCard>
              ) : fileDiff ? (
                <DiffViewer diff={fileDiff} />
              ) : (
                <GlassCard>
                  <div className="text-center py-12 text-zinc-500">
                    No diff available for this file
                  </div>
                </GlassCard>
              )}
            </div>
          ) : (
            <GlassCard>
              <div className="text-center py-12">
                <Eye className="w-12 h-12 text-zinc-600 mx-auto mb-4" />
                <p className="text-zinc-400">Select a file to view its diff</p>
              </div>
            </GlassCard>
          )}
        </div>
      </div>

      {/* Last Sync Info */}
      {syncStatus.last_sync && (
        <div className="mt-6 text-center text-sm text-zinc-500">
          Last synced: {new Date(syncStatus.last_sync).toLocaleString()}
        </div>
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}

// Summary Card Component
interface SummaryCardProps {
  label: string
  value: number
  icon: React.ReactNode
  color?: 'green' | 'yellow' | 'blue' | 'purple' | 'red'
  active?: boolean
  onClick?: () => void
}

function SummaryCard({ label, value, icon, color, active, onClick }: SummaryCardProps) {
  const colorClasses = {
    green: 'text-green-400',
    yellow: 'text-yellow-400',
    blue: 'text-blue-400',
    purple: 'text-purple-400',
    red: 'text-red-400',
  }

  return (
    <button
      onClick={onClick}
      className={`p-4 rounded-xl transition-all ${
        active
          ? 'bg-primary-500/20 border-2 border-primary-500/50'
          : 'bg-white/5 border-2 border-transparent hover:bg-white/10'
      }`}
    >
      <div className={`mb-2 ${color ? colorClasses[color] : 'text-zinc-400'}`}>
        {icon}
      </div>
      <div className="text-2xl font-bold text-white">{value}</div>
      <div className="text-xs text-zinc-400">{label}</div>
    </button>
  )
}
