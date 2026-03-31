import { useEffect, useState } from 'react'
import { Link, useNavigate, useSearchParams, useParams } from 'react-router-dom'
import { ArrowLeft, Search, ChevronRight, Home, Plus, Clock, Filter, GitMerge, RefreshCw, CheckCircle } from 'lucide-react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import TreeView from '../components/TreeView'
import ContentView from '../components/ContentView'
import WorkspaceSwitcher from '../components/WorkspaceSwitcher'
import RevisionBrowser from '../components/RevisionBrowser'
import RevisionCompareView from '../components/RevisionCompareView'
import TimeTravelBanner from '../components/TimeTravelBanner'
import CommitDialog from '../components/CommitDialog'
import CreateTagDialog from '../components/CreateTagDialog'
import QueryBuilder from '../components/QueryBuilder'
import LanguageSwitcher from '../components/LanguageSwitcher'
import { useRepositoryContext } from '../hooks/useRepositoryContext'
import { nodesApi, Node as NodeType } from '../api/nodes'
import { branchesApi, BranchDivergence, Branch } from '../api/branches'
import { repositoriesApi } from '../api/repositories'
import MergeBranchDialog from '../components/MergeBranchDialog'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'

type PendingCommit = {
  type: 'update' | 'create' | 'delete'
  data: any
  node?: NodeType
  parent?: NodeType | null
}

export default function ContentExplorer() {
  const { repo, branch, workspace } = useRepositoryContext()
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const params = useParams()

  const [nodes, setNodes] = useState<NodeType[]>([])
  const [selectedNode, setSelectedNode] = useState<NodeType | null>(null)
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(true)
  const [searchTerm, setSearchTerm] = useState('')
  const [showEditor, setShowEditor] = useState(false)
  const [showMobileTree, setShowMobileTree] = useState(true)
  const [isMobile, setIsMobile] = useState(false)
  const [showRevisionBrowser, setShowRevisionBrowser] = useState(false)
  const [pendingCommit, setPendingCommit] = useState<PendingCommit | null>(null)
  const [showCreateTag, setShowCreateTag] = useState<string | null>(null)
  const [compareRevisions, setCompareRevisions] = useState<[string, string] | null>(null)
  const [showQueryBuilder, setShowQueryBuilder] = useState(false)
  const [queryResults, setQueryResults] = useState<NodeType[] | null>(null)
  const [currentLocale, setCurrentLocale] = useState<string | null>(null)
  const [branchDivergence, setBranchDivergence] = useState<BranchDivergence | null>(null)
  const [mainBranch, setMainBranch] = useState<string | null>(null)
  const [currentBranchInfo, setCurrentBranchInfo] = useState<Branch | null>(null)
  const [showMergeDialog, setShowMergeDialog] = useState(false)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, closeToast } = useToast()

  // Read revision from URL query params (keep as string, don't parse to number)
  const currentRevision = searchParams.get('rev')
  const isTimeTravelMode = currentRevision !== null

  // Extract path from URL (the * wildcard parameter)
  const urlPath = params['*'] ? `/${params['*']}` : null

  // Detect mobile viewport
  useEffect(() => {
    const checkMobile = () => {
      setIsMobile(window.innerWidth < 768)
    }
    checkMobile()
    window.addEventListener('resize', checkMobile)
    return () => window.removeEventListener('resize', checkMobile)
  }, [])

  useEffect(() => {
    // Reset tree state when revision or locale changes
    setExpandedNodes(new Set())
    setSelectedNode(null)
    // Clear all loaded children data to force fresh fetch at new revision/locale
    setNodes([])
    loadRootNodes()
  }, [repo, branch, workspace, currentRevision, currentLocale])

  function handleLocaleChange(locale: string) {
    setCurrentLocale(locale)
    // The useEffect above will automatically reload nodes when currentLocale changes
  }

  // Fetch repository config to get the default (main) branch
  useEffect(() => {
    async function fetchMainBranch() {
      try {
        const repoInfo = await repositoriesApi.get(repo)
        setMainBranch(repoInfo.config.default_branch)
      } catch (error) {
        console.error('Failed to fetch repository config:', error)
      }
    }
    fetchMainBranch()
  }, [repo])

  // Fetch current branch info to get upstream_branch
  useEffect(() => {
    async function fetchBranchInfo() {
      if (!branch || !repo) {
        setCurrentBranchInfo(null)
        return
      }
      try {
        const branchInfo = await branchesApi.get(repo, branch)
        setCurrentBranchInfo(branchInfo)
      } catch (error) {
        console.error('Failed to fetch branch info:', error)
        setCurrentBranchInfo(null)
      }
    }
    fetchBranchInfo()
  }, [repo, branch])

  // Fetch branch divergence when not on main branch and not in time travel mode
  // Uses upstream_branch if set, otherwise falls back to mainBranch
  useEffect(() => {
    async function fetchDivergence() {
      if (!branch || !mainBranch || branch === mainBranch || isTimeTravelMode) {
        setBranchDivergence(null)
        return
      }

      try {
        // Use upstream_branch if set, otherwise compare against main
        const baseBranch = currentBranchInfo?.upstream_branch || mainBranch
        const divergence = await branchesApi.compare(repo, branch, baseBranch)
        setBranchDivergence(divergence)
      } catch (error) {
        console.error('Failed to fetch branch divergence:', error)
        setBranchDivergence(null)
      }
    }
    fetchDivergence()
  }, [repo, branch, mainBranch, isTimeTravelMode, currentBranchInfo])

  async function loadRootNodes() {
    try {
      setLoading(true)
      const data = currentRevision !== null
        ? await nodesApi.listRootAtRevision(repo, branch, workspace, currentRevision, currentLocale || undefined)
        : await nodesApi.listRootAtHead(repo, branch, workspace, currentLocale || undefined)
      setNodes(data)
    } catch (error) {
      console.error('Failed to load nodes:', error)
    } finally {
      setLoading(false)
    }
  }

  async function loadNodeChildren(node: NodeType) {
    try {
      // If children already loaded as objects, just expand
      if (node.children && node.children.length > 0 && typeof node.children[0] === 'object') {
        setExpandedNodes(prev => new Set(prev).add(node.id))
        return
      }

      // Fetch children with full details
      const childDetails = currentRevision !== null
        ? await nodesApi.listChildrenAtRevision(repo, branch, workspace, node.path, currentRevision, currentLocale || undefined)
        : await nodesApi.listChildrenAtHead(repo, branch, workspace, node.path, currentLocale || undefined)

      // Update the nodes tree with children
      const updateNodeInTree = (nodes: NodeType[]): NodeType[] => {
        return nodes.map(n => {
          if (n.id === node.id) {
            return { ...n, children: childDetails }
          }
          if (n.children && Array.isArray(n.children)) {
            return { ...n, children: updateNodeInTree(n.children as NodeType[]) }
          }
          return n
        })
      }

      setNodes(updateNodeInTree(nodes))
      setExpandedNodes(prev => new Set(prev).add(node.id))
    } catch (error) {
      console.error('Failed to load children:', error)
    }
  }

  // Handle URL path changes - expand tree and select node
  useEffect(() => {
    if (!urlPath || loading || nodes.length === 0) return

    const pathToLoad = urlPath // Capture in closure to satisfy TypeScript

    async function selectNodeFromPath() {
      try {
        // Fetch the target node by path
        const targetNode = currentRevision !== null
          ? await nodesApi.getAtRevision(repo, branch, workspace, pathToLoad, currentRevision, currentLocale || undefined)
          : await nodesApi.getAtHead(repo, branch, workspace, pathToLoad, currentLocale || undefined)

        if (!targetNode) return

        // Split path to get all parent paths
        const pathParts = pathToLoad.split('/').filter(Boolean)
        const parentPaths: string[] = []

        // Build list of parent paths (e.g., /blog, /blog/posts)
        for (let i = 0; i < pathParts.length - 1; i++) {
          parentPaths.push('/' + pathParts.slice(0, i + 1).join('/'))
        }

        // Load and expand all parent nodes
        for (const parentPath of parentPaths) {
          try {
            const parentNode = currentRevision !== null
              ? await nodesApi.getAtRevision(repo, branch, workspace, parentPath, currentRevision, currentLocale || undefined)
              : await nodesApi.getAtHead(repo, branch, workspace, parentPath, currentLocale || undefined)

            if (parentNode) {
              await loadNodeChildren(parentNode)
            }
          } catch (e) {
            console.error(`Failed to load parent node ${parentPath}:`, e)
          }
        }

        // Select the target node
        setSelectedNode(targetNode)
      } catch (error) {
        console.error('Failed to select node from URL path:', error)
      }
    }

    selectNodeFromPath()
  }, [urlPath, loading, nodes.length, repo, branch, workspace, currentRevision, currentLocale])

  function handleNodeClick(node: NodeType) {
    setSelectedNode(node)
    // Update URL to reflect the selected node path
    const newPath = `/${repo}/content/${branch}/${workspace}${node.path}`
    navigate(newPath + (currentRevision ? `?rev=${currentRevision}` : ''))
  }

  function handleNodeExpand(node: NodeType) {
    if (expandedNodes.has(node.id)) {
      // Collapse
      setExpandedNodes(prev => {
        const next = new Set(prev)
        next.delete(node.id)
        return next
      })
    } else {
      // Expand - load children if needed
      loadNodeChildren(node)
    }
  }

  async function handleNodeUpdate(updatedNode: Partial<NodeType>) {
    if (!selectedNode || isTimeTravelMode) return

    setPendingCommit({
      type: 'update',
      data: updatedNode,
      node: selectedNode,
    })
  }

  async function executeCommit(message: string, actor: string) {
    if (!pendingCommit) return

    try {
      // Create commit metadata for backend
      const commit = { message, actor }
      
      if (pendingCommit.type === 'update' && pendingCommit.node) {
        // For UPDATE: only send properties (and translations if present)
        const updatePayload: any = {
          commit,
        }
        
        if (pendingCommit.data.properties) {
          updatePayload.properties = pendingCommit.data.properties
        }
        
        if (pendingCommit.data.translations) {
          updatePayload.translations = pendingCommit.data.translations
        }
        
        await nodesApi.update(repo, branch, workspace, pendingCommit.node.path, updatePayload)
        await loadRootNodes()
        const fullNode = await nodesApi.getAtHead(repo, branch, workspace, pendingCommit.node.path, currentLocale || undefined)
        setSelectedNode(fullNode)
      } else if (pendingCommit.type === 'create') {
        // For CREATE: send name, node_type, properties, and commit
        const createData = { ...pendingCommit.data, commit }
        if (pendingCommit.parent) {
          await nodesApi.create(repo, branch, workspace, pendingCommit.parent.path, createData)
        } else {
          await nodesApi.createRoot(repo, branch, workspace, createData)
        }
        await loadRootNodes()
      } else if (pendingCommit.type === 'delete' && pendingCommit.node) {
        // For DELETE: only send commit metadata
        await nodesApi.delete(repo, branch, workspace, pendingCommit.node.path, { commit })
        setSelectedNode(null)
        await loadRootNodes()
      }
      setPendingCommit(null)
    } catch (error) {
      console.error('Failed to commit:', error)
      throw error
    }
  }

  async function handleNodeDelete(node: NodeType) {
    if (isTimeTravelMode) return

    setDeleteConfirm({
      message: `Delete "${node.name}"? This cannot be undone.`,
      onConfirm: () => {
        setPendingCommit({
          type: 'delete',
          data: {},
          node,
        })
      }
    })
  }

  async function handleNodePublish(node: NodeType) {
    if (isTimeTravelMode) return
    try {
      await nodesApi.publish(repo, branch, workspace, node.path)
      await loadRootNodes()
      if (selectedNode?.id === node.id) {
        const updated = await nodesApi.getAtHead(repo, branch, workspace, node.path, currentLocale || undefined)
        setSelectedNode(updated)
      }
    } catch (error) {
      console.error('Failed to publish node:', error)
    }
  }

  async function handleNodeUnpublish(node: NodeType) {
    if (isTimeTravelMode) return
    try {
      await nodesApi.unpublish(repo, branch, workspace, node.path)
      await loadRootNodes()
      if (selectedNode?.id === node.id) {
        const updated = await nodesApi.getAtHead(repo, branch, workspace, node.path, currentLocale || undefined)
        setSelectedNode(updated)
      }
    } catch (error) {
      console.error('Failed to unpublish node:', error)
    }
  }

  async function handleCreateChild(parent: NodeType | null, nodeData: any) {
    if (isTimeTravelMode) return
    setPendingCommit({
      type: 'create',
      data: nodeData,
      parent,
    })
  }

  async function handleNodeCopy(node: NodeType, destination: string, newName?: string, recursive?: boolean) {
    if (isTimeTravelMode) return
    try {
      const copyMethod = recursive ? nodesApi.copyTree : nodesApi.copy
      const action = recursive ? 'Copy tree' : 'Copy'

      await copyMethod(repo, branch, workspace, node.path, {
        destination,
        name: newName,
        commit: {
          message: `${action} ${node.name} to ${destination}`,
          actor: 'user' // TODO: Get actual user from auth context
        }
      })
      await loadRootNodes()
    } catch (error) {
      console.error('Failed to copy node:', error)
      throw error
    }
  }

  async function handleNodeMove(node: NodeType, targetPath: string) {
    if (isTimeTravelMode) return
    try {
      await nodesApi.move(repo, branch, workspace, node.path, {
        destination: targetPath,
        commit: {
          message: `Move ${node.name} from ${node.path} to ${targetPath}`,
          actor: 'user' // TODO: Get actual user from auth context
        }
      })
      await loadRootNodes()
      if (selectedNode?.id === node.id) {
        setSelectedNode(null)
      }
    } catch (error) {
      console.error('Failed to move node:', error)
      throw error
    }
  }

  function handleSelectRevision(revision: string) {
    // Navigate to same route with revision query param
    const params = new URLSearchParams()
    params.set('rev', revision)
    navigate(`/${repo}/content/${branch}/${workspace}?${params.toString()}`)
    setShowRevisionBrowser(false)
  }

  function handleExitTimeTravel() {
    // Navigate back to HEAD by removing revision param
    navigate(`/${repo}/content/${branch}/${workspace}`)
  }

  function handleCreateTag(revision: string) {
    setShowCreateTag(revision)
  }

  function handleTagCreated() {
    setShowCreateTag(null)
    // Could reload revisions here if needed
  }

  function handleCompareRevisions(from: string, to: string) {
    // Store both revisions without comparison logic (HLC strings can't be compared with < operator)
    setCompareRevisions([from, to])
    setShowRevisionBrowser(false)
  }

  function handleExitCompareMode() {
    setCompareRevisions(null)
  }

  async function handleExecuteQuery(query: any) {
    try {
      const results = await nodesApi.queryDsl(repo, branch, workspace, query)
      setQueryResults(results)
      setShowQueryBuilder(false)
    } catch (error) {
      console.error('Failed to execute query:', error)
      showError('Error', 'Query execution failed. Check console for details.')
    }
  }

  function handleClearQueryResults() {
    setQueryResults(null)
  }

  const breadcrumbs = selectedNode ? selectedNode.path.split('/').filter(Boolean) : []

  return (
    <div className="h-screen flex flex-col bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black">
      {/* Time Travel Banner */}
      {isTimeTravelMode && (
        <TimeTravelBanner
          revision={currentRevision}
          onExitTimeTravel={handleExitTimeTravel}
        />
      )}
      
      {/* Header */}
      <div className="flex-shrink-0 bg-black/30 backdrop-blur-md border-b border-white/10">
        <div className="px-4 md:px-6 py-3 md:py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 md:gap-4 min-w-0 flex-1">
              <Link
                to={`/${repo}`}
                className="inline-flex items-center gap-1 md:gap-2 text-primary-400 hover:text-primary-300 flex-shrink-0"
              >
                <ArrowLeft className="w-4 h-4" />
                <span className="hidden sm:inline">Back</span>
              </Link>
              <span className="text-gray-500 hidden sm:inline">/</span>
              <Link to={`/${repo}`} className="text-white font-semibold text-sm md:text-base truncate hover:text-primary-300">
                {repo}
              </Link>
              <span className="text-gray-500 hidden sm:inline">/</span>

              {/* Workspace Switcher */}
              <WorkspaceSwitcher className="hidden md:flex" />

              {/* Branch Divergence Badge - show when there's divergence OR in sync */}
              {branchDivergence && !isMobile && (
                branchDivergence.ahead === 0 && branchDivergence.behind === 0 ? (
                  /* In Sync Badge */
                  <div className="flex items-center gap-1.5 px-3 py-1.5 bg-green-500/10 border border-green-500/20 rounded-lg text-sm text-green-400">
                    <CheckCircle className="w-3.5 h-3.5" />
                    <span>In Sync</span>
                  </div>
                ) : (
                  /* Divergence Badge */
                  <div className="flex items-center gap-2 px-3 py-1.5 bg-white/5 border border-white/10 rounded-lg text-sm">
                    {branchDivergence.behind > 0 && (
                      <span className="flex items-center gap-1 text-amber-400">
                        <ArrowLeft className="w-3 h-3" />
                        {branchDivergence.behind} behind
                      </span>
                    )}
                    {branchDivergence.behind > 0 && branchDivergence.ahead > 0 && (
                      <span className="text-gray-500">|</span>
                    )}
                    {branchDivergence.ahead > 0 && (
                      <span className="flex items-center gap-1 text-green-400">
                        <ChevronRight className="w-3 h-3" />
                        {branchDivergence.ahead} ahead
                      </span>
                    )}
                  </div>
                )
              )}

              {/* Sync Button - show when behind upstream/main branch */}
              {!isMobile && branchDivergence && branchDivergence.behind > 0 && mainBranch && !isTimeTravelMode && (
                <button
                  onClick={() => setShowMergeDialog(true)}
                  className="flex items-center gap-1.5 px-3 py-1.5 bg-blue-500/20 hover:bg-blue-500/30 border border-blue-500/30 rounded-lg text-sm text-blue-400 transition-colors"
                  title={`Update ${branch} from ${currentBranchInfo?.upstream_branch || mainBranch}`}
                >
                  <RefreshCw className="w-3.5 h-3.5" />
                  <span className="hidden lg:inline">Update from {currentBranchInfo?.upstream_branch || mainBranch}</span>
                  <span className="lg:hidden">Sync</span>
                </button>
              )}

              {/* Merge Button - only show when there's divergence (not when in sync) */}
              {!isMobile && branch && mainBranch && !isTimeTravelMode && branchDivergence && (branchDivergence.ahead > 0 || branchDivergence.behind > 0) && (
                <button
                  onClick={() => setShowMergeDialog(true)}
                  className="flex items-center gap-2 px-3 py-1.5 bg-primary-500/20 border border-primary-400/30 rounded-lg text-primary-300 hover:bg-primary-500/30 transition-colors text-sm"
                  title="Merge branches (Experimental)"
                >
                  <GitMerge className="w-4 h-4" />
                  <span className="hidden lg:inline">Merge</span>
                </button>
              )}
            </div>

            <div className="flex items-center gap-2">
              {/* Language Switcher */}
              {!isMobile && (
                <LanguageSwitcher
                  onLocaleChange={handleLocaleChange}
                  compact={false}
                />
              )}

              {/* Query Builder Toggle */}
              {!isMobile && (
                <button
                  onClick={() => setShowQueryBuilder(!showQueryBuilder)}
                  className={`px-3 py-2 rounded-lg transition-colors flex items-center gap-2 ${
                    showQueryBuilder
                      ? 'bg-primary-500 text-white'
                      : 'bg-white/10 text-white/80 hover:bg-white/20'
                  }`}
                >
                  <Filter className="w-4 h-4" />
                  <span className="hidden lg:inline">Query</span>
                </button>
              )}

              {/* Revision Browser Toggle */}
              {!isMobile && (
                <button
                  onClick={() => setShowRevisionBrowser(!showRevisionBrowser)}
                  className={`px-3 py-2 rounded-lg transition-colors flex items-center gap-2 ${
                    showRevisionBrowser
                      ? 'bg-primary-500 text-white'
                      : 'bg-white/10 text-white/80 hover:bg-white/20'
                  }`}
                >
                  <Clock className="w-4 h-4" />
                  <span className="hidden lg:inline">History</span>
                </button>
              )}

              {/* Mobile toggle button */}
              {isMobile && (
                <button
                  onClick={() => setShowMobileTree(!showMobileTree)}
                  className="flex items-center gap-2 px-3 py-1.5 bg-primary-500/20 border border-primary-400/30 rounded-lg text-primary-300 text-sm"
                >
                  {showMobileTree ? 'Show Content' : 'Show Tree'}
                </button>
              )}
            </div>

            {/* Breadcrumb navigation - Desktop only */}
            {selectedNode && !isMobile && (
              <div className="flex items-center gap-1 text-sm">
                <button
                  onClick={() => setSelectedNode(null)}
                  className="text-gray-400 hover:text-white p-1"
                >
                  <Home className="w-4 h-4" />
                </button>
                {breadcrumbs.map((part, index) => (
                  <div key={index} className="flex items-center gap-1">
                    <ChevronRight className="w-4 h-4 text-gray-500" />
                    <span className={index === breadcrumbs.length - 1 ? 'text-white' : 'text-gray-400'}>
                      {part}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Main content area */}
      <div className="flex-1 overflow-hidden">
        {isMobile ? (
          /* Mobile: Toggle between tree and content */
          <div className="h-full">
            {showMobileTree ? (
              /* Tree view */
              <div className="h-full bg-black/30 backdrop-blur-md flex flex-col">
                {/* Toolbar */}
                <div className="p-4 border-b border-white/10 space-y-3">
                  <button
                    onClick={() => {
                      setSelectedNode(null)
                      setShowEditor(true)
                      setShowMobileTree(false)
                    }}
                    className="w-full px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors flex items-center justify-center gap-2"
                  >
                    <Plus className="w-4 h-4" />
                    Create Root Node
                  </button>
                  <div className="relative">
                    <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-gray-400" />
                    <input
                      type="text"
                      placeholder="Search nodes..."
                      value={searchTerm}
                      onChange={(e) => setSearchTerm(e.target.value)}
                      className="w-full pl-10 pr-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white placeholder-zinc-400 focus:outline-none focus:ring-2 focus:ring-primary-500"
                    />
                  </div>
                </div>

                {/* Tree view */}
                <div className="flex-1 overflow-auto p-4">
                  {loading ? (
                    <div className="text-center text-gray-400 py-8">Loading...</div>
                  ) : (
                    <TreeView
                      nodes={nodes}
                      expandedNodes={expandedNodes}
                      selectedNodeId={selectedNode?.id}
                      onNodeClick={(node) => {
                        handleNodeClick(node)
                        setShowMobileTree(false) // Switch to content view
                      }}
                      onNodeExpand={handleNodeExpand}
                      onEdit={(node) => {
                        setSelectedNode(node)
                        setShowEditor(true)
                        setShowMobileTree(false)
                      }}
                      onAddChild={(node) => {
                        setSelectedNode(node)
                        setShowMobileTree(false)
                      }}
                      onDelete={handleNodeDelete}
                      onPublish={handleNodePublish}
                      onUnpublish={handleNodeUnpublish}
                      onCopy={(node) => {
                        setSelectedNode(node)
                        setShowMobileTree(false)
                      }}
                      onMove={(node) => {
                        setSelectedNode(node)
                        setShowMobileTree(false)
                      }}
                      onCreateRoot={() => {
                        setSelectedNode(null)
                        setShowEditor(true)
                        setShowMobileTree(false)
                      }}
                    />
                  )}
                </div>
              </div>
            ) : (
              /* Content view or Comparison view */
              compareRevisions ? (
                <RevisionCompareView
                  repo={repo!}
                  branch={branch!}
                  workspace={workspace!}
                  fromRevision={compareRevisions[0]}
                  toRevision={compareRevisions[1]}
                  onClose={handleExitCompareMode}
                />
              ) : (
                <ContentView
                  repo={repo!}
                  branch={branch!}
                  workspace={workspace!}
                  node={selectedNode}
                  allNodes={nodes}
                  showEditor={showEditor}
                  currentLocale={currentLocale}
                  onUpdate={handleNodeUpdate}
                  onDelete={handleNodeDelete}
                  onPublish={handleNodePublish}
                  onUnpublish={handleNodeUnpublish}
                  onCreateChild={handleCreateChild}
                  onCopy={handleNodeCopy}
                  onMove={handleNodeMove}
                  onCloseEditor={() => setShowEditor(false)}
                  onTranslationUpdate={loadRootNodes}
                  readonly={isTimeTravelMode}
                />
              )
            )}
          </div>
        ) : (
          /* Desktop: Split panes */
          <Allotment>
            {/* Left sidebar - Tree view */}
            <Allotment.Pane minSize={200} preferredSize={350}>
              <div className="h-full bg-black/30 backdrop-blur-md border-r border-white/10 flex flex-col">
                {/* Toolbar */}
                <div className="p-4 border-b border-white/10 space-y-3">
                  <button
                    onClick={() => {
                      setSelectedNode(null)
                      setShowEditor(true)
                    }}
                    className="w-full px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors flex items-center justify-center gap-2"
                  >
                    <Plus className="w-4 h-4" />
                    Create Root Node
                  </button>
                  <div className="relative">
                    <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-gray-400" />
                    <input
                      type="text"
                      placeholder="Search nodes..."
                      value={searchTerm}
                      onChange={(e) => setSearchTerm(e.target.value)}
                      className="w-full pl-10 pr-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white placeholder-zinc-400 focus:outline-none focus:ring-2 focus:ring-primary-500"
                    />
                  </div>
                </div>

                {/* Tree view or Query Results */}
                <div className="flex-1 overflow-auto p-4">
                  {queryResults ? (
                    /* Query Results View */
                    <div className="space-y-3">
                      <div className="flex items-center justify-between pb-2 border-b border-white/10">
                        <div>
                          <h3 className="text-sm font-semibold text-white">Query Results</h3>
                          <p className="text-xs text-gray-400">{queryResults.length} nodes found</p>
                        </div>
                        <button
                          onClick={handleClearQueryResults}
                          className="px-3 py-1.5 bg-white/10 hover:bg-white/20 text-white text-sm rounded-lg transition-colors"
                        >
                          Clear
                        </button>
                      </div>
                      {queryResults.map((node) => (
                        <div
                          key={node.id}
                          onClick={() => handleNodeClick(node)}
                          className="p-3 bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg cursor-pointer transition-colors"
                        >
                          <div className="font-medium text-white text-sm">{node.name}</div>
                          <div className="text-xs text-gray-400 mt-1">{node.path}</div>
                          <div className="text-xs text-primary-400 mt-1">{node.node_type}</div>
                        </div>
                      ))}
                    </div>
                  ) : loading ? (
                    <div className="text-center text-gray-400 py-8">Loading...</div>
                  ) : (
                    <TreeView
                      nodes={nodes}
                      expandedNodes={expandedNodes}
                      selectedNodeId={selectedNode?.id}
                      onNodeClick={handleNodeClick}
                      onNodeExpand={handleNodeExpand}
                      onEdit={(node) => {
                        setSelectedNode(node)
                        setShowEditor(true)
                      }}
                      onAddChild={(node) => {
                        setSelectedNode(node)
                      }}
                      onDelete={handleNodeDelete}
                      onPublish={handleNodePublish}
                      onUnpublish={handleNodeUnpublish}
                      onCopy={(node) => {
                        setSelectedNode(node)
                      }}
                      onMove={(node) => {
                        setSelectedNode(node)
                      }}
                      onCreateRoot={() => {
                        setSelectedNode(null)
                        setShowEditor(true)
                      }}
                    />
                  )}
                </div>
              </div>
            </Allotment.Pane>

            {/* Right panel - Content view or Comparison view */}
            <Allotment.Pane>
              {compareRevisions ? (
                <RevisionCompareView
                  repo={repo!}
                  branch={branch!}
                  workspace={workspace!}
                  fromRevision={compareRevisions[0]}
                  toRevision={compareRevisions[1]}
                  onClose={handleExitCompareMode}
                />
              ) : (
                <ContentView
                  repo={repo!}
                  branch={branch!}
                  workspace={workspace!}
                  node={selectedNode}
                  allNodes={nodes}
                  showEditor={showEditor}
                  currentLocale={currentLocale}
                  onUpdate={handleNodeUpdate}
                  onDelete={handleNodeDelete}
                  onPublish={handleNodePublish}
                  onUnpublish={handleNodeUnpublish}
                  onCreateChild={handleCreateChild}
                  onCopy={handleNodeCopy}
                  onMove={handleNodeMove}
                  onCloseEditor={() => setShowEditor(false)}
                  onTranslationUpdate={loadRootNodes}
                  readonly={isTimeTravelMode}
                />
              )}
            </Allotment.Pane>
            
            {/* Revision Browser Panel */}
            {showRevisionBrowser && (
              <Allotment.Pane minSize={300} preferredSize={400}>
                <RevisionBrowser
                  repo={repo}
                  branch={branch}
                  workspace={workspace}
                  onSelectRevision={handleSelectRevision}
                  onCompareRevisions={handleCompareRevisions}
                  onCreateTag={handleCreateTag}
                />
              </Allotment.Pane>
            )}
          </Allotment>
        )}
      </div>

      {/* Dialogs */}
      {pendingCommit && (
        <CommitDialog
          title={
            pendingCommit.type === 'create'
              ? 'Create Node'
              : pendingCommit.type === 'update'
              ? 'Update Node'
              : 'Delete Node'
          }
          action={
            pendingCommit.type === 'create'
              ? `Creating "${pendingCommit.data.name}"`
              : pendingCommit.type === 'update'
              ? `Updating "${pendingCommit.node?.name}"`
              : `Deleting "${pendingCommit.node?.name}"`
          }
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}

      {showCreateTag !== null && (
        <CreateTagDialog
          repoId={repo}
          defaultRevision={showCreateTag}
          onClose={() => setShowCreateTag(null)}
          onSuccess={handleTagCreated}
        />
      )}

      {showQueryBuilder && (
        <QueryBuilder
          onExecute={handleExecuteQuery}
          onClose={() => setShowQueryBuilder(false)}
        />
      )}

      {/* Merge Branch Dialog */}
      {showMergeDialog && branch && mainBranch && (
        <MergeBranchDialog
          open={showMergeDialog}
          onClose={() => setShowMergeDialog(false)}
          currentBranch={branch}
          mainBranch={mainBranch}
          repoId={repo}
          onMergeComplete={async () => {
            // Reload the tree after merge
            await loadRootNodes()
            // Refresh branch divergence info (using upstream_branch if set)
            if (branch !== mainBranch) {
              try {
                const baseBranch = currentBranchInfo?.upstream_branch || mainBranch
                const divergence = await branchesApi.compare(repo, branch, baseBranch)
                setBranchDivergence(divergence)
              } catch (error) {
                console.error('Failed to refresh divergence:', error)
              }
            }
          }}
        />
      )}
      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Confirm Deletion"
        message={deleteConfirm?.message || ''}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          deleteConfirm?.onConfirm()
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
