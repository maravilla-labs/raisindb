import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import * as LucideIcons from 'lucide-react'
import {
  Package,
  ArrowLeft,
  Download,
  Trash2,
  CheckCircle,
  XCircle,
  FileCode,
  Folder,
  Box,
  Zap,
  History,
  Eye,
  RefreshCw,
  AlertTriangle,
  Merge,
  GitBranch,
  FileArchive,
  GitCompare,
} from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import ConfirmDialog from '../../components/ConfirmDialog'
import { useToast, ToastContainer } from '../../components/Toast'
import MarkdownRenderer from '../../components/MarkdownRenderer'
import SplitButton, { type SplitButtonOption } from '../../components/SplitButton'
import ExportPackageDialog from '../../components/ExportPackageDialog'
import DryRunDialog from '../../components/DryRunDialog'
import { packagesApi, type PackageDetails as PackageDetailsType, type InstallMode } from '../../api/packages'
import { jobsApi, type JobEventData } from '../../api/jobs'
import { branchesApi, type Branch } from '../../api/branches'

// Helper to check if icon is a URL
const isIconUrl = (icon: string): boolean => {
  return icon.startsWith('http://') || icon.startsWith('https://') || icon.startsWith('/')
}

// Convert kebab-case to PascalCase for Lucide component lookup
const iconNameToPascalCase = (name: string): string => {
  return name
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('')
}

// Get Lucide icon component by name
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const getIconComponent = (name: string): any => {
  const pascalName = iconNameToPascalCase(name)
  return (LucideIcons as any)[pascalName] || Package
}

// Install mode options for the split button (Install button - not yet installed)
const installModeOptions: SplitButtonOption<InstallMode>[] = [
  {
    value: 'sync',
    label: 'Install (Sync)',
    description: 'Update existing content with package content, add new content, leave untouched content alone.',
    icon: <Merge className="w-4 h-4" />,
  },
  {
    value: 'skip',
    label: 'Install (Skip)',
    description: 'Only install content to paths that do not already exist. Preserves existing content.',
    icon: <Download className="w-4 h-4" />,
  },
  {
    value: 'overwrite',
    label: 'Install (Overwrite)',
    description: 'Delete and replace all existing content from this package. Use with caution.',
    icon: <AlertTriangle className="w-4 h-4" />,
  },
]

// Reinstall mode options (package already installed)
const reinstallModeOptions: SplitButtonOption<InstallMode>[] = [
  {
    value: 'sync',
    label: 'Reinstall (Sync)',
    description: 'Update existing content with package content, add new content, leave untouched content alone.',
    icon: <Merge className="w-4 h-4" />,
  },
  {
    value: 'skip',
    label: 'Reinstall (Skip)',
    description: 'Only install content to paths that do not already exist. Preserves existing content.',
    icon: <Download className="w-4 h-4" />,
  },
  {
    value: 'overwrite',
    label: 'Reinstall (Overwrite)',
    description: 'Delete and replace all existing content from this package. Use with caution.',
    icon: <AlertTriangle className="w-4 h-4" />,
  },
]

export default function PackageDetails() {
  const navigate = useNavigate()
  // Use wildcard param since route is :branch/packages/* not :branch/packages/:name
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const name = pathParam // The package path (e.g., "ai-tools" or "folder/ai-tools")
  const [pkg, setPkg] = useState<PackageDetailsType | null>(null)
  const [loading, setLoading] = useState(true)
  const [actionLoading, setActionLoading] = useState(false)
  const [deleteConfirm, setDeleteConfirm] = useState(false)
  const [installConfirm, setInstallConfirm] = useState(false)
  const [pendingInstallMode, setPendingInstallMode] = useState<InstallMode | null>(null)
  const [installJobId, setInstallJobId] = useState<string | null>(null)
  const [installProgress, setInstallProgress] = useState<number | null>(null)
  const [installStatus, setInstallStatus] = useState<string | null>(null)
  const [branches, setBranches] = useState<Branch[]>([])
  const [selectedBranch, setSelectedBranch] = useState<string>(branch || 'main')
  const [readmeContent, setReadmeContent] = useState<string | null>(null)
  const [readmeLoading, setReadmeLoading] = useState(false)
  const [exportDialogOpen, setExportDialogOpen] = useState(false)
  const [dryRunDialogOpen, setDryRunDialogOpen] = useState(false)
  const [dryRunMode, setDryRunMode] = useState<InstallMode>('sync')
  const { toasts, error: showError, success: showSuccess, info: showInfo, closeToast } = useToast()

  useEffect(() => {
    loadPackage()
  }, [repo, name])

  // Fetch README.md content from package child node
  useEffect(() => {
    async function loadReadme() {
      if (!repo || !pkg?.name) return
      setReadmeLoading(true)
      try {
        // README is stored as an Asset child node: /packages/{pkg.name}/README.md
        // Fetch via @file to get the raw markdown content
        const response = await fetch(
          `/api/repository/${encodeURIComponent(repo)}/${encodeURIComponent(selectedBranch)}/head/packages/${encodeURIComponent(pkg.name)}/README.md@file`
        )
        if (response.ok) {
          const content = await response.text()
          setReadmeContent(content)
        } else {
          // Fallback to legacy readme property if child node doesn't exist
          setReadmeContent(pkg.readme || null)
        }
      } catch (error) {
        console.error('Failed to load README:', error)
        setReadmeContent(pkg.readme || null)
      } finally {
        setReadmeLoading(false)
      }
    }
    loadReadme()
  }, [repo, pkg?.name, pkg?.readme, selectedBranch])

  // Load available branches for the branch selector
  useEffect(() => {
    async function loadBranches() {
      if (!repo) return
      try {
        const branchList = await branchesApi.list(repo)
        setBranches(branchList)
        // Default to 'main' if it exists, otherwise first branch
        const mainBranch = branchList.find((b) => b.name === 'main')
        if (mainBranch) {
          setSelectedBranch('main')
        } else if (branchList.length > 0) {
          setSelectedBranch(branchList[0].name)
        }
      } catch (error) {
        console.error('Failed to load branches:', error)
        // Keep default 'main' on error
      }
    }
    loadBranches()
  }, [repo])

  // Subscribe to job events when we have an active install job
  useEffect(() => {
    if (!installJobId) return

    const cleanup = jobsApi.subscribeToJobEvents((event: JobEventData) => {
      // Only handle events for our install job
      if (event.job_id !== installJobId) return

      // Build status message with retry info if available
      let statusMessage = event.status
      if (event.retry_count > 0 && event.status === 'Scheduled') {
        statusMessage = `Retrying (${event.retry_count}/${event.max_retries})`
        // Show error toast on first retry detection
        if (event.error) {
          showError('Install Error', `Retrying: ${event.error}`)
        }
      } else if (event.status === 'Running') {
        statusMessage = 'Installing...'
      }

      setInstallStatus(statusMessage)
      if (event.progress !== undefined) {
        setInstallProgress(Math.round(event.progress * 100))
      }

      // Check if job completed
      if (event.status === 'Completed') {
        showSuccess('Installed', `Package "${pkg?.title || pkg?.name}" installed successfully`)
        setInstallJobId(null)
        setInstallProgress(null)
        setInstallStatus(null)
        setActionLoading(false)
        loadPackage()
      } else if (event.status.startsWith('Failed')) {
        const errorMsg = event.error || event.status.replace('Failed: ', '')
        showError('Install Failed', errorMsg)
        setInstallJobId(null)
        setInstallProgress(null)
        setInstallStatus(null)
        setActionLoading(false)
      }
    })

    return cleanup
  }, [installJobId, pkg])

  async function loadPackage() {
    if (!repo || !name) return
    setLoading(true)
    try {
      const data = await packagesApi.getPackage(repo, decodeURIComponent(name))
      setPkg(data)
    } catch (error) {
      console.error('Failed to load package:', error)
      showError('Load Failed', 'Failed to load package details')
    } finally {
      setLoading(false)
    }
  }

  async function handleInstall(mode: 'skip' | 'overwrite' | 'sync' = 'skip') {
    if (!repo || !name || !pkg) return
    setActionLoading(true)
    const isReinstall = pkg.installed && mode !== 'skip'
    const actionLabel = isReinstall ? 'Reinstalling' : 'Installing'
    const successLabel = isReinstall ? 'Reinstalled' : 'Installed'

    try {
      const response = await packagesApi.installPackage(repo, pkg.name, { mode, branch: selectedBranch })

      // If already installed and using skip mode, no job was created
      if (response.installed && mode === 'skip') {
        showInfo('Already Installed', `Package "${pkg.title || pkg.name}" is already installed`)
        setActionLoading(false)
        loadPackage()
        return
      }

      // If we got a job_id, track the installation progress
      if (response.job_id) {
        setInstallJobId(response.job_id)
        setInstallProgress(0)
        setInstallStatus('Starting...')
        showInfo(actionLabel, `${actionLabel} package "${pkg.title || pkg.name}"...`)
        // Don't setActionLoading(false) here - we'll do it when job completes
      } else {
        // No job_id means synchronous install (shouldn't happen with new backend)
        showSuccess(successLabel, `Package "${pkg.title || pkg.name}" ${successLabel.toLowerCase()} successfully`)
        setActionLoading(false)
        loadPackage()
      }
    } catch (error) {
      console.error(`Failed to ${isReinstall ? 'reinstall' : 'install'} package:`, error)
      showError(`${actionLabel} Failed`, `Failed to start package ${isReinstall ? 'reinstallation' : 'installation'}`)
      setActionLoading(false)
    }
  }

  async function handleUninstall() {
    if (!repo || !name || !pkg) return
    setActionLoading(true)
    try {
      await packagesApi.uninstallPackage(repo, pkg.name)
      showSuccess('Uninstalled', `Package "${pkg.title || pkg.name}" uninstalled successfully`)
      loadPackage()
    } catch (error) {
      console.error('Failed to uninstall package:', error)
      showError('Uninstall Failed', 'Failed to uninstall package')
    } finally {
      setActionLoading(false)
    }
  }

  async function handleDelete() {
    if (!repo || !name || !pkg) return
    try {
      await packagesApi.deletePackage(repo, pkg.name)
      showSuccess('Deleted', `Package "${pkg.title || pkg.name}" deleted successfully`)
      navigate(`/${repo}/packages`)
    } catch (error) {
      console.error('Failed to delete package:', error)
      showError('Delete Failed', 'Failed to delete package')
    }
  }

  function confirmInstall(mode: InstallMode) {
    setPendingInstallMode(mode)
    setInstallConfirm(true)
  }

  function handleBrowseContents() {
    if (!pkg) return
    navigate(`/${repo}/packages/${encodeURIComponent(pkg.name)}/browse`)
  }

  function handleDryRunPreview(mode: InstallMode) {
    setDryRunMode(mode)
    setDryRunDialogOpen(true)
  }

  function handleDryRunProceed() {
    // When user clicks "Proceed with Install" in DryRunDialog,
    // open the install confirmation dialog with the same mode
    setPendingInstallMode(dryRunMode)
    setInstallConfirm(true)
  }

  // Get description for the selected install mode
  function getInstallModeDescription(mode: InstallMode): string {
    switch (mode) {
      case 'sync':
        return 'Update existing content, add new content, and leave untouched content alone.'
      case 'skip':
        return 'Only install content to paths that do not exist. Existing content will be preserved.'
      case 'overwrite':
        return 'Delete and replace all existing content from this package.'
    }
  }

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="text-center text-zinc-400 py-12">Loading...</div>
      </div>
    )
  }

  if (!pkg) {
    return (
      <div className="animate-fade-in">
        <GlassCard>
          <div className="text-center py-12">
            <Package className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">Package not found</h3>
            <p className="text-zinc-400">The package you're looking for doesn't exist</p>
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
          onClick={() => navigate(`/${repo}/packages`)}
          className="flex items-center gap-2 text-zinc-400 hover:text-white mb-4 transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Packages
        </button>

        <div className="flex items-start gap-6">
          {pkg.icon && isIconUrl(pkg.icon) ? (
            // URL-based icon - render as image
            <div className="w-20 h-20 flex-shrink-0 rounded-xl overflow-hidden bg-white/10">
              <img src={pkg.icon} alt={pkg.name} className="w-full h-full object-cover" />
            </div>
          ) : pkg.color ? (
            // Custom color provided - use inline styles
            (() => {
              const IconComponent = pkg.icon ? getIconComponent(pkg.icon) : Package
              return (
                <div
                  className="w-20 h-20 flex-shrink-0 rounded-xl flex items-center justify-center"
                  style={{ backgroundColor: `${pkg.color}20` }}
                >
                  <IconComponent className="w-10 h-10" style={{ color: pkg.color }} />
                </div>
              )
            })()
          ) : (
            // Default - use Tailwind primary colors
            (() => {
              const IconComponent = pkg.icon ? getIconComponent(pkg.icon) : Package
              return (
                <div className="w-20 h-20 flex-shrink-0 rounded-xl bg-primary-500/20 flex items-center justify-center">
                  <IconComponent className="w-10 h-10 text-primary-400" />
                </div>
              )
            })()
          )}

          <div className="flex-1">
            <div className="flex items-start justify-between mb-2">
              <div>
                <h1 className="text-4xl font-bold text-white mb-2">
                  {pkg.title || pkg.name}
                </h1>
                <p className="text-zinc-400">v{pkg.version}</p>
              </div>
              <div className="flex items-center gap-2">
                {pkg.installed ? (
                  <>
                    <SplitButton
                      options={reinstallModeOptions}
                      defaultValue="sync"
                      onSelect={confirmInstall}
                      loading={actionLoading}
                      loadingLabel={installJobId ? 'Reinstalling' : 'Starting...'}
                      loadingProgress={installProgress}
                      loadingStatus={installStatus}
                      disabled={actionLoading}
                      variant="primary"
                      icon={<RefreshCw className="w-5 h-5" />}
                    />
                    <SplitButton
                      options={reinstallModeOptions.map(opt => ({
                        ...opt,
                        label: opt.label.replace('Reinstall', 'Preview'),
                        description: `Preview what ${opt.label.toLowerCase().replace('reinstall', 'reinstalling')} would do`,
                      }))}
                      defaultValue="sync"
                      onSelect={handleDryRunPreview}
                      disabled={actionLoading}
                      variant="secondary"
                      icon={<Eye className="w-5 h-5" />}
                    />
                    <button
                      onClick={handleUninstall}
                      disabled={actionLoading}
                      className="flex items-center gap-2 px-4 py-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors disabled:opacity-50"
                    >
                      <XCircle className="w-5 h-5" />
                      Uninstall
                    </button>
                  </>
                ) : (
                  <>
                    <SplitButton
                      options={installModeOptions}
                      defaultValue="sync"
                      onSelect={confirmInstall}
                      loading={actionLoading}
                      loadingLabel={installJobId ? 'Installing' : 'Starting...'}
                      loadingProgress={installProgress}
                      loadingStatus={installStatus}
                      disabled={actionLoading}
                      variant="success"
                      icon={<Download className="w-5 h-5" />}
                    />
                    <SplitButton
                      options={installModeOptions.map(opt => ({
                        ...opt,
                        label: opt.label.replace('Install', 'Preview'),
                        description: `Preview what ${opt.label.toLowerCase().replace('install', 'installing')} would do`,
                      }))}
                      defaultValue="sync"
                      onSelect={handleDryRunPreview}
                      disabled={actionLoading}
                      variant="secondary"
                      icon={<Eye className="w-5 h-5" />}
                    />
                  </>
                )}
                <button
                  onClick={() => navigate(`/${repo}/${selectedBranch}/packages/${encodeURIComponent(pkg?.name || '')}/sync`)}
                  className="flex items-center gap-2 px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
                >
                  <GitCompare className="w-5 h-5" />
                  Sync Status
                </button>
                <button
                  onClick={() => setExportDialogOpen(true)}
                  className="flex items-center gap-2 px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
                >
                  <FileArchive className="w-5 h-5" />
                  Export
                </button>
                <button
                  onClick={() => setDeleteConfirm(true)}
                  className="flex items-center gap-2 px-4 py-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors"
                >
                  <Trash2 className="w-5 h-5" />
                  Delete
                </button>
              </div>
            </div>

            {pkg.author && (
              <p className="text-sm text-zinc-400 mb-2">by {pkg.author}</p>
            )}

            {pkg.installed && (
              <div className="flex items-center gap-2 px-3 py-1.5 bg-green-500/20 text-green-400 rounded-lg inline-flex">
                <CheckCircle className="w-4 h-4" />
                <span className="text-sm font-medium">Installed</span>
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Main Content */}
        <div className="lg:col-span-2 space-y-6">
          {/* Description */}
          {pkg.description && (
            <GlassCard>
              <h2 className="text-xl font-semibold text-white mb-4">Description</h2>
              <p className="text-zinc-300 whitespace-pre-wrap">{pkg.description}</p>
            </GlassCard>
          )}

          {/* Browse Contents */}
          <GlassCard>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <FileCode className="w-5 h-5 text-primary-400" />
                <h2 className="text-xl font-semibold text-white">Package Contents</h2>
              </div>
              <button
                onClick={handleBrowseContents}
                className="flex items-center gap-2 px-3 py-1.5 bg-primary-500/20 hover:bg-primary-500/30 text-primary-400 rounded-lg transition-colors text-sm"
              >
                <Eye className="w-4 h-4" />
                Browse Files
              </button>
            </div>
          </GlassCard>

          {/* README */}
          {(readmeContent || readmeLoading) && (
            <GlassCard>
              <h2 className="text-xl font-semibold text-white mb-4">README</h2>
              {readmeLoading ? (
                <div className="text-zinc-400">Loading README...</div>
              ) : readmeContent ? (
                <MarkdownRenderer
                  content={readmeContent}
                  assetBaseUrl={`/api/repository/${encodeURIComponent(repo || '')}/${encodeURIComponent(selectedBranch)}/head/packages/${encodeURIComponent(pkg?.name || '')}`}
                />
              ) : null}
            </GlassCard>
          )}

          {/* Changelog */}
          {pkg.changelog && (
            <GlassCard>
              <div className="flex items-center gap-2 mb-4">
                <History className="w-5 h-5 text-primary-400" />
                <h2 className="text-xl font-semibold text-white">Changelog</h2>
              </div>
              <div className="prose prose-invert prose-sm max-w-none">
                <pre className="bg-black/20 p-4 rounded-lg overflow-auto text-sm text-zinc-300">
                  {pkg.changelog}
                </pre>
              </div>
            </GlassCard>
          )}
        </div>

        {/* Sidebar */}
        <div className="space-y-6">
          {/* Metadata */}
          <GlassCard>
            <h3 className="text-lg font-semibold text-white mb-4">Metadata</h3>
            <div className="space-y-3 text-sm">
              <div>
                <span className="text-zinc-500">Name:</span>
                <p className="text-white font-mono">{pkg.name}</p>
              </div>
              <div>
                <span className="text-zinc-500">Version:</span>
                <p className="text-white">{pkg.version}</p>
              </div>
              {pkg.category && (
                <div>
                  <span className="text-zinc-500">Category:</span>
                  <p className="text-white">{pkg.category}</p>
                </div>
              )}
              {pkg.created_at && (
                <div>
                  <span className="text-zinc-500">Created:</span>
                  <p className="text-white">
                    {new Date(pkg.created_at).toLocaleDateString()}
                  </p>
                </div>
              )}
              {pkg.updated_at && (
                <div>
                  <span className="text-zinc-500">Updated:</span>
                  <p className="text-white">
                    {new Date(pkg.updated_at).toLocaleDateString()}
                  </p>
                </div>
              )}
            </div>
          </GlassCard>

          {/* Keywords */}
          {pkg.keywords && pkg.keywords.length > 0 && (
            <GlassCard>
              <h3 className="text-lg font-semibold text-white mb-4">Keywords</h3>
              <div className="flex flex-wrap gap-2">
                {pkg.keywords.map((keyword) => (
                  <span
                    key={keyword}
                    className="px-3 py-1 bg-white/10 text-zinc-300 text-sm rounded-full"
                  >
                    {keyword}
                  </span>
                ))}
              </div>
            </GlassCard>
          )}

          {/* Dependencies */}
          {pkg.dependencies && pkg.dependencies.length > 0 && (
            <GlassCard>
              <div className="flex items-center gap-2 mb-4">
                <Box className="w-5 h-5 text-primary-400" />
                <h3 className="text-lg font-semibold text-white">Dependencies</h3>
              </div>
              <div className="space-y-2">
                {pkg.dependencies.map((dep) => (
                  <div
                    key={dep.name}
                    className="flex items-center justify-between p-2 bg-white/5 rounded"
                  >
                    <span className="text-white text-sm">{dep.name}</span>
                    <span className="text-zinc-400 text-xs">
                      {dep.version}
                      {dep.optional && (
                        <span className="ml-2 text-zinc-500">(optional)</span>
                      )}
                    </span>
                  </div>
                ))}
              </div>
            </GlassCard>
          )}

          {/* Provides */}
          {pkg.provides && (
            <GlassCard>
              <div className="flex items-center gap-2 mb-4">
                <Zap className="w-5 h-5 text-primary-400" />
                <h3 className="text-lg font-semibold text-white">Provides</h3>
              </div>
              <div className="space-y-3 text-sm">
                {pkg.provides.node_types && pkg.provides.node_types.length > 0 && (
                  <div>
                    <span className="text-zinc-500 block mb-2">Node Types:</span>
                    <div className="space-y-1">
                      {pkg.provides.node_types.map((nt) => (
                        <div
                          key={nt}
                          className="px-2 py-1 bg-white/5 rounded text-white font-mono text-xs"
                        >
                          {nt}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
                {pkg.provides.workspaces && pkg.provides.workspaces.length > 0 && (
                  <div>
                    <span className="text-zinc-500 block mb-2">Workspaces:</span>
                    <div className="space-y-1">
                      {pkg.provides.workspaces.map((ws) => (
                        <div
                          key={ws}
                          className="flex items-center gap-2 px-2 py-1 bg-white/5 rounded text-white text-xs"
                        >
                          <Folder className="w-3 h-3" />
                          {ws}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
                {pkg.provides.functions && pkg.provides.functions.length > 0 && (
                  <div>
                    <span className="text-zinc-500 block mb-2">Functions:</span>
                    <div className="space-y-1">
                      {pkg.provides.functions.map((fn) => (
                        <div
                          key={fn}
                          className="px-2 py-1 bg-white/5 rounded text-white font-mono text-xs"
                        >
                          {fn}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
                {pkg.provides.content && pkg.provides.content.length > 0 && (
                  <div>
                    <span className="text-zinc-500 block mb-2">Content:</span>
                    <div className="space-y-1">
                      {pkg.provides.content.map((c) => (
                        <div
                          key={c}
                          className="px-2 py-1 bg-white/5 rounded text-white text-xs"
                        >
                          {c}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </GlassCard>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={deleteConfirm}
        title="Delete Package"
        message={`Are you sure you want to delete "${pkg.title || pkg.name}"? This action cannot be undone.`}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          handleDelete()
          setDeleteConfirm(false)
        }}
        onCancel={() => setDeleteConfirm(false)}
      />

      <ConfirmDialog
        open={installConfirm}
        title={pkg.installed ? 'Reinstall Package' : 'Install Package'}
        message={`${pkg.installed ? 'Reinstall' : 'Install'} "${pkg.title || pkg.name}" using ${pendingInstallMode?.toUpperCase()} mode?\n\n${pendingInstallMode ? getInstallModeDescription(pendingInstallMode) : ''}`}
        variant={pendingInstallMode === 'overwrite' ? 'danger' : 'info'}
        confirmText={pkg.installed ? 'Reinstall' : 'Install'}
        onConfirm={() => {
          if (pendingInstallMode) {
            handleInstall(pendingInstallMode)
          }
          setInstallConfirm(false)
          setPendingInstallMode(null)
        }}
        onCancel={() => {
          setInstallConfirm(false)
          setPendingInstallMode(null)
        }}
      >
        {/* Branch Selector */}
        <div className="flex items-center gap-3 p-3 bg-white/5 rounded-lg">
          <GitBranch className="w-5 h-5 text-zinc-400" />
          <div className="flex-1">
            <label htmlFor="branch-select" className="block text-sm text-zinc-400 mb-1">
              Target Branch
            </label>
            <select
              id="branch-select"
              value={selectedBranch}
              onChange={(e) => setSelectedBranch(e.target.value)}
              className="w-full bg-black/30 border border-white/10 text-white rounded-lg px-3 py-2 focus:outline-none focus:ring-2 focus:ring-primary-500/50"
            >
              {branches.map((branch) => (
                <option key={branch.name} value={branch.name}>
                  {branch.name}
                  {branch.protected ? ' (protected)' : ''}
                </option>
              ))}
              {branches.length === 0 && (
                <option value="main">main</option>
              )}
            </select>
          </div>
        </div>
      </ConfirmDialog>

      <ExportPackageDialog
        open={exportDialogOpen}
        repo={repo || ''}
        packageName={pkg?.name || ''}
        branch={selectedBranch}
        onClose={() => setExportDialogOpen(false)}
        onSuccess={() => {
          showSuccess('Export Ready', 'Package exported successfully')
        }}
      />

      <DryRunDialog
        open={dryRunDialogOpen}
        repo={repo || ''}
        packageName={pkg?.name || ''}
        packageTitle={pkg?.title}
        branch={selectedBranch}
        mode={dryRunMode}
        onClose={() => setDryRunDialogOpen(false)}
        onProceed={handleDryRunProceed}
      />

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
