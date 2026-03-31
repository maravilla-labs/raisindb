import { useState, useEffect, useCallback } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { ArrowLeft, Package, CheckCircle, Loader2, Database, ChevronDown, RefreshCw } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import SelectableTreeView, { SelectedPath as TreeSelectedPath } from '../../components/SelectableTreeView'
import { useToast, ToastContainer } from '../../components/Toast'
import { createPackageFromSelection } from '../../api/packages'
import { workspacesApi, Workspace } from '../../api/workspaces'
import { nodesApi, Node } from '../../api/nodes'
import { sseManager, JobEvent } from '../../api/management'

type CreateStatus = 'idle' | 'creating' | 'complete' | 'error'

export default function PackageCreate() {
  const navigate = useNavigate()
  const { repo, branch: urlBranch } = useParams<{ repo: string; branch: string }>()
  const activeBranch = urlBranch || 'main'

  // Form state
  const [packageName, setPackageName] = useState('')
  const [packageVersion, setPackageVersion] = useState('1.0.0')
  const [packageTitle, setPackageTitle] = useState('')
  const [packageDescription, setPackageDescription] = useState('')
  const [packageAuthor, setPackageAuthor] = useState('')
  const [includeNodeTypes, setIncludeNodeTypes] = useState(true)

  // Workspace and tree state
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [selectedWorkspace, setSelectedWorkspace] = useState<string>('')
  const [nodes, setNodes] = useState<Node[]>([])
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())
  const [selectedPaths, setSelectedPaths] = useState<Map<string, TreeSelectedPath>>(new Map())
  const [loadingWorkspaces, setLoadingWorkspaces] = useState(true)
  const [loadingNodes, setLoadingNodes] = useState(false)

  // Create status
  const [createStatus, setCreateStatus] = useState<CreateStatus>('idle')
  const [createProgress, setCreateProgress] = useState(0)
  const [statusMessage, setStatusMessage] = useState('')
  const [trackingJobId, setTrackingJobId] = useState<string | null>(null)
  const [downloadPath, setDownloadPath] = useState<string | null>(null)

  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  // Load workspaces on mount
  useEffect(() => {
    async function loadWorkspaces() {
      if (!repo) return
      try {
        setLoadingWorkspaces(true)
        const ws = await workspacesApi.list(repo)
        // Filter out system workspaces
        const userWorkspaces = ws.filter(w =>
          !['packages', 'nodetypes', 'admin', 'system'].includes(w.name)
        )
        setWorkspaces(userWorkspaces)
        if (userWorkspaces.length > 0 && !selectedWorkspace) {
          // Default to 'content' if available, otherwise first workspace
          const defaultWs = userWorkspaces.find(w => w.name === 'content') || userWorkspaces[0]
          setSelectedWorkspace(defaultWs.name)
        }
      } catch (error) {
        console.error('Failed to load workspaces:', error)
        showError('Error', 'Failed to load workspaces')
      } finally {
        setLoadingWorkspaces(false)
      }
    }
    loadWorkspaces()
  }, [repo, showError])

  // Load nodes when workspace changes
  useEffect(() => {
    async function loadNodes() {
      if (!repo || !selectedWorkspace) return
      try {
        setLoadingNodes(true)
        const rootNodes = await nodesApi.listRootAtHead(repo, activeBranch, selectedWorkspace)
        setNodes(rootNodes)
        setExpandedNodes(new Set())
      } catch (error) {
        console.error('Failed to load nodes:', error)
        showError('Error', 'Failed to load workspace content')
      } finally {
        setLoadingNodes(false)
      }
    }
    loadNodes()
  }, [repo, activeBranch, selectedWorkspace, showError])

  // Handle node expansion
  const handleNodeExpand = useCallback(async (node: Node) => {
    if (!repo || !selectedWorkspace) return

    const newExpanded = new Set(expandedNodes)
    if (newExpanded.has(node.id)) {
      newExpanded.delete(node.id)
    } else {
      newExpanded.add(node.id)
      // Load children if not already loaded
      if (!node.children || node.children.length === 0) {
        try {
          const nodeWithChildren = await nodesApi.getAtHead(repo, activeBranch, selectedWorkspace, node.path)
          // Update the nodes array with the loaded children
          const updateNodeChildren = (nodeList: Node[]): Node[] => {
            return nodeList.map(n => {
              if (n.id === node.id) {
                return { ...n, children: nodeWithChildren.children }
              }
              if (n.children) {
                return { ...n, children: updateNodeChildren(n.children) }
              }
              return n
            })
          }
          setNodes(prevNodes => updateNodeChildren(prevNodes))
        } catch (error) {
          console.error('Failed to load children:', error)
        }
      }
    }
    setExpandedNodes(newExpanded)
  }, [repo, activeBranch, selectedWorkspace, expandedNodes])

  // SSE connection for job progress tracking
  useEffect(() => {
    if (!trackingJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        if (event.job_id !== trackingJobId) return

        if (event.progress !== undefined && event.progress !== null) {
          setCreateProgress(Math.round(event.progress * 100))
        }

        if (event.status === 'Completed') {
          setCreateStatus('complete')
          setCreateProgress(100)
          setStatusMessage('Package created successfully!')
          showSuccess('Success', 'Package created successfully')
        } else if (event.status.startsWith('Failed')) {
          setCreateStatus('error')
          const errorMsg = event.error || event.status.replace('Failed: ', '')
          setStatusMessage(`Creation failed: ${errorMsg}`)
          showError('Failed', errorMsg)
        } else if (event.status === 'Running') {
          setStatusMessage('Creating package...')
        }
      },
      onError: () => {
        console.error('SSE connection error for job tracking')
      }
    })

    return cleanup
  }, [trackingJobId, showSuccess, showError])

  // Handle form submission
  async function handleCreate() {
    if (!repo || !packageName.trim()) {
      showError('Validation Error', 'Package name is required')
      return
    }

    if (selectedPaths.size === 0) {
      showError('Validation Error', 'Please select at least one item to include in the package')
      return
    }

    setCreateStatus('creating')
    setCreateProgress(0)
    setStatusMessage('Starting package creation...')

    try {
      // Convert selected paths to API format
      const paths = Array.from(selectedPaths.values()).map(p => ({
        workspace: p.workspace,
        path: p.isRecursive ? `${p.path}/*` : p.path
      }))

      const response = await createPackageFromSelection(repo, {
        name: packageName.trim(),
        version: packageVersion.trim() || '1.0.0',
        selected_paths: paths,
        include_node_types: includeNodeTypes,
        title: packageTitle.trim() || undefined,
        description: packageDescription.trim() || undefined,
        author: packageAuthor.trim() || undefined
      }, activeBranch)

      setTrackingJobId(response.job_id)
      setDownloadPath(response.download_path)
      setStatusMessage('Creating package...')
    } catch (error) {
      console.error('Failed to create package:', error)
      setCreateStatus('error')
      setStatusMessage(error instanceof Error ? error.message : 'Failed to create package')
      showError('Error', error instanceof Error ? error.message : 'Failed to create package')
    }
  }

  const isCreating = createStatus === 'creating'
  const isComplete = createStatus === 'complete'
  const isError = createStatus === 'error'
  const canCreate = packageName.trim() && selectedPaths.size > 0 && !isCreating && !isComplete

  // Calculate selected count across all workspaces
  const totalSelected = selectedPaths.size

  return (
    <div className="animate-fade-in">
      {/* Header */}
      <div className="mb-8">
        <button
          onClick={() => navigate(`/${repo}/${activeBranch}/packages`)}
          className="flex items-center gap-2 text-zinc-400 hover:text-white mb-4 transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Packages
        </button>

        <h1 className="text-4xl font-bold text-white mb-2">Create Package</h1>
        <p className="text-zinc-400">
          Create a new package from selected content
        </p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Left Column: Content Selection */}
        <div className="space-y-6">
          <GlassCard>
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-semibold text-white flex items-center gap-2">
                <Database className="w-5 h-5 text-primary-400" />
                Select Content
              </h2>
              <span className="text-sm text-zinc-400">
                {totalSelected} item{totalSelected !== 1 ? 's' : ''} selected
              </span>
            </div>

            {/* Workspace Selector */}
            <div className="mb-4">
              <label className="block text-sm font-medium text-zinc-400 mb-2">
                Workspace
              </label>
              {loadingWorkspaces ? (
                <div className="flex items-center gap-2 text-zinc-400">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Loading workspaces...
                </div>
              ) : (
                <div className="relative">
                  <select
                    value={selectedWorkspace}
                    onChange={(e) => setSelectedWorkspace(e.target.value)}
                    className="w-full appearance-none bg-white/5 border border-white/10 rounded-lg px-4 py-2 text-white focus:outline-none focus:border-primary-500 pr-10"
                  >
                    {workspaces.map((ws) => (
                      <option key={ws.name} value={ws.name} className="bg-zinc-900">
                        {ws.name}
                      </option>
                    ))}
                  </select>
                  <ChevronDown className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-400 pointer-events-none" />
                </div>
              )}
            </div>

            {/* Tree View */}
            <div className="border border-white/10 rounded-lg p-4 max-h-96 overflow-y-auto bg-black/20">
              {loadingNodes ? (
                <div className="flex items-center justify-center py-8 text-zinc-400">
                  <Loader2 className="w-5 h-5 animate-spin mr-2" />
                  Loading content...
                </div>
              ) : (
                <SelectableTreeView
                  nodes={nodes}
                  workspace={selectedWorkspace}
                  expandedNodes={expandedNodes}
                  selectedPaths={selectedPaths}
                  onNodeExpand={handleNodeExpand}
                  onSelectionChange={setSelectedPaths}
                  allowRecursiveSelection={true}
                />
              )}
            </div>

            {/* Quick actions */}
            <div className="flex gap-2 mt-4">
              <button
                onClick={() => setSelectedPaths(new Map())}
                className="text-sm text-zinc-400 hover:text-white transition-colors"
              >
                Clear Selection
              </button>
              <span className="text-zinc-600">|</span>
              <button
                onClick={() => {
                  setExpandedNodes(new Set())
                  setNodes([...nodes])
                }}
                className="text-sm text-zinc-400 hover:text-white transition-colors flex items-center gap-1"
              >
                <RefreshCw className="w-3 h-3" />
                Refresh
              </button>
            </div>
          </GlassCard>
        </div>

        {/* Right Column: Package Details */}
        <div className="space-y-6">
          <GlassCard>
            <h2 className="text-lg font-semibold text-white flex items-center gap-2 mb-4">
              <Package className="w-5 h-5 text-primary-400" />
              Package Details
            </h2>

            <div className="space-y-4">
              {/* Package Name */}
              <div>
                <label className="block text-sm font-medium text-zinc-400 mb-2">
                  Package Name <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  value={packageName}
                  onChange={(e) => setPackageName(e.target.value)}
                  placeholder="my-package"
                  className="w-full bg-white/5 border border-white/10 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500"
                />
                <p className="text-xs text-zinc-500 mt-1">
                  Alphanumeric characters, hyphens, and underscores only
                </p>
              </div>

              {/* Version */}
              <div>
                <label className="block text-sm font-medium text-zinc-400 mb-2">
                  Version
                </label>
                <input
                  type="text"
                  value={packageVersion}
                  onChange={(e) => setPackageVersion(e.target.value)}
                  placeholder="1.0.0"
                  className="w-full bg-white/5 border border-white/10 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500"
                />
              </div>

              {/* Title */}
              <div>
                <label className="block text-sm font-medium text-zinc-400 mb-2">
                  Title
                </label>
                <input
                  type="text"
                  value={packageTitle}
                  onChange={(e) => setPackageTitle(e.target.value)}
                  placeholder="My Package"
                  className="w-full bg-white/5 border border-white/10 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500"
                />
              </div>

              {/* Description */}
              <div>
                <label className="block text-sm font-medium text-zinc-400 mb-2">
                  Description
                </label>
                <textarea
                  value={packageDescription}
                  onChange={(e) => setPackageDescription(e.target.value)}
                  placeholder="A brief description of what this package contains..."
                  rows={3}
                  className="w-full bg-white/5 border border-white/10 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500 resize-none"
                />
              </div>

              {/* Author */}
              <div>
                <label className="block text-sm font-medium text-zinc-400 mb-2">
                  Author
                </label>
                <input
                  type="text"
                  value={packageAuthor}
                  onChange={(e) => setPackageAuthor(e.target.value)}
                  placeholder="Your Name"
                  className="w-full bg-white/5 border border-white/10 rounded-lg px-4 py-2 text-white placeholder-zinc-500 focus:outline-none focus:border-primary-500"
                />
              </div>

              {/* Include Node Types */}
              <div className="flex items-center gap-3">
                <button
                  onClick={() => setIncludeNodeTypes(!includeNodeTypes)}
                  className={`w-5 h-5 rounded border-2 flex items-center justify-center transition-colors ${
                    includeNodeTypes
                      ? 'bg-primary-500 border-primary-500'
                      : 'border-zinc-500 hover:border-zinc-400'
                  }`}
                >
                  {includeNodeTypes && <CheckCircle className="w-3 h-3 text-white" />}
                </button>
                <span className="text-zinc-300">Include node type definitions</span>
              </div>
            </div>
          </GlassCard>

          {/* Progress/Status */}
          {(isCreating || isComplete || isError) && (
            <GlassCard>
              <div className="flex items-center gap-3 mb-4">
                {isCreating && <Loader2 className="w-5 h-5 text-primary-400 animate-spin" />}
                {isComplete && <CheckCircle className="w-5 h-5 text-green-400" />}
                {isError && <Package className="w-5 h-5 text-red-400" />}
                <span className={`font-medium ${
                  isComplete ? 'text-green-400' : isError ? 'text-red-400' : 'text-white'
                }`}>
                  {statusMessage}
                </span>
              </div>

              {isCreating && (
                <div className="w-full bg-white/10 rounded-full h-2 overflow-hidden">
                  <div
                    className="bg-primary-500 h-full transition-all duration-300"
                    style={{ width: `${createProgress}%` }}
                  />
                </div>
              )}

              {isComplete && downloadPath && (
                <a
                  href={downloadPath}
                  className="inline-flex items-center gap-2 mt-4 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                >
                  <Package className="w-4 h-4" />
                  Download Package
                </a>
              )}
            </GlassCard>
          )}

          {/* Action Buttons */}
          <div className="flex gap-4">
            <button
              onClick={handleCreate}
              disabled={!canCreate}
              className="flex-1 flex items-center justify-center gap-2 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isCreating ? (
                <>
                  <Loader2 className="w-5 h-5 animate-spin" />
                  Creating...
                </>
              ) : isComplete ? (
                <>
                  <CheckCircle className="w-5 h-5" />
                  Complete
                </>
              ) : (
                <>
                  <Package className="w-5 h-5" />
                  Create Package
                </>
              )}
            </button>

            {!isCreating && !isComplete && (
              <button
                onClick={() => navigate(`/${repo}/${activeBranch}/packages`)}
                className="px-6 py-3 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
              >
                Cancel
              </button>
            )}
          </div>
        </div>
      </div>

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
