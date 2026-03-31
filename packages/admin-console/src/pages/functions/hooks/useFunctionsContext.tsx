/**
 * Functions IDE context and state management
 */

import { createContext, useContext, useState, useCallback, useEffect, useMemo, type ReactNode } from 'react'
import { useNavigate, useParams, useLocation } from 'react-router-dom'
import { nodesApi, type Node as NodeType } from '../../../api/nodes'
import { functionsApi } from '../../../api/functions'
import { workspacesApi } from '../../../api/workspaces'
import { useRepositoryContext } from '../../../hooks/useRepositoryContext'
import type { FunctionNode, EditorTab, LogEntry, ExecutionRecord, FunctionLanguage, ValidationProblem } from '../types'
import { usePreferences, type UsePreferencesReturn } from './usePreferences'

const FUNCTIONS_WORKSPACE = 'functions'


interface CommitInfo {
  message: string
  actor: string
}

interface FunctionsContextValue extends UsePreferencesReturn {
  // Repository context
  repo: string
  branch: string
  workspace: string

  // Tree state
  nodes: NodeType[]
  expandedNodes: Set<string>
  selectedNode: FunctionNode | null
  loading: boolean

  // Editor state
  openTabs: EditorTab[]
  activeTabId: string | null
  codeCache: Map<string, string>

  // Output state
  logs: LogEntry[]
  executions: ExecutionRecord[]
  executionsLoading: boolean
  problems: ValidationProblem[]

  // Rename state
  renamingNodeId: string | null

  // Tree actions
  loadRootNodes: () => Promise<void>
  loadNodeChildren: (node: NodeType) => Promise<void>
  selectNode: (node: FunctionNode | null) => void
  expandNode: (nodeId: string) => void
  collapseNode: (nodeId: string) => void

  // Rename actions
  setRenamingNodeId: (nodeId: string | null) => void
  renameNode: (node: NodeType, newName: string, commit: CommitInfo) => Promise<void>
  deleteNode: (node: NodeType, commit: CommitInfo) => Promise<void>
  moveNode: (node: NodeType, destinationPath: string, commit: CommitInfo) => Promise<void>
  reorderNode: (node: NodeType, targetPath: string, position: 'before' | 'after', commit: CommitInfo) => Promise<void>

  // Creation rules
  getCreationOptions: (parentNode: NodeType | null) => { canCreateFolder: boolean; canCreateFunction: boolean; canCreateFile: boolean; canCreateAgent: boolean }

  // Tab actions
  openTab: (node: NodeType) => void
  closeTab: (tabId: string) => void
  setActiveTab: (tabId: string) => void
  markTabDirty: (tabId: string, isDirty: boolean) => void
  updateTabPath: (tabId: string, newPath: string) => void

  // Code actions
  getCode: (path: string) => string | undefined
  setCode: (path: string, code: string) => void
  loadCode: (node: NodeType) => Promise<{ code: string; filePath: string } | null>

  // Log actions
  addLog: (log: LogEntry) => void
  clearLogs: () => void
  addExecution: (execution: ExecutionRecord) => void
  updateExecution: (executionId: string, update: Partial<ExecutionRecord>) => void
  clearExecutions: () => void
  loadExecutions: () => Promise<void>

  // Problems actions
  setProblems: (problems: ValidationProblem[]) => void
  clearProblems: () => void
}

const FunctionsContext = createContext<FunctionsContextValue | null>(null)

export function FunctionsProvider({ children }: { children: ReactNode }) {
  const { repo, branch } = useRepositoryContext()
  const navigate = useNavigate()
  const params = useParams()
  const location = useLocation()
  const preferencesHook = usePreferences()
  const { setExpandedFolders } = preferencesHook

  const normalizePath = useCallback((rawPath: string | null) => {
    if (!rawPath) return null
    const withSlash = rawPath.startsWith('/') ? rawPath : `/${rawPath}`
    const decoded = (() => {
      try { return decodeURIComponent(withSlash) } catch { return withSlash }
    })()
    // Collapse duplicated trailing extensions like ".js.js"
    return decoded.replace(/(\.[^/.]+)(\1)+(\/?)$/, '$1$3')
  }, [])

  const rawWildcardPath = params['*'] ? `/${params['*']}` : null
  const urlPath = useMemo(() => normalizePath(rawWildcardPath), [rawWildcardPath, normalizePath])

  // Tree state
  const [nodes, setNodes] = useState<NodeType[]>([])
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set(preferencesHook.preferences.expandedFolders))
  const [selectedNode, setSelectedNode] = useState<FunctionNode | null>(null)
  const [loading, setLoading] = useState(true)

  // Editor state
  const [openTabs, setOpenTabs] = useState<EditorTab[]>([])
  const [activeTabId, setActiveTabId] = useState<string | null>(null)
  const [codeCache] = useState<Map<string, string>>(new Map())

  // Output state
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [executions, setExecutions] = useState<ExecutionRecord[]>([])
  const [executionsLoading, setExecutionsLoading] = useState(false)
  const [problems, setProblemsState] = useState<ValidationProblem[]>([])

  // Rename state
  const [renamingNodeId, setRenamingNodeId] = useState<string | null>(null)

  // Workspace metadata - used to check allowed node types
  const [workspaceAllowedTypes, setWorkspaceAllowedTypes] = useState<string[]>([])

  // Normalize wildcard path to avoid duplicated extensions (e.g., ".js.js")
  const normalizedUrlPath = useMemo(() => urlPath, [urlPath])

  // Load root nodes on mount
  const loadRootNodes = useCallback(async () => {
    console.log("Loading root function nodes for repo:", repo, "branch:", branch)
    if (!repo || !branch) {
      setLoading(false) // Ensure we turn off the loading state! Otherwise it will prevent the UI from updating and navigating to other routes.
      return
    }

    try {
      setLoading(true)
      const data = await nodesApi.listRootAtHead(repo, branch, FUNCTIONS_WORKSPACE)
      setNodes(data)
    } catch (error) {
      console.error('Failed to load function nodes:', error)
    } finally {
      setLoading(false)
    }
  }, [repo, branch])

  // Load children of a node
  const loadNodeChildren = useCallback(
    async (node: NodeType) => {
      if (!repo || !branch) return

      // If children already loaded as objects, just return
      if (node.children && node.children.length > 0 && typeof node.children[0] === 'object') {
        return
      }

      try {
        const childDetails = await nodesApi.listChildrenAtHead(repo, branch, FUNCTIONS_WORKSPACE, node.path)

        // Update the nodes tree with children
        const updateNodeInTree = (nodes: NodeType[]): NodeType[] => {
          return nodes.map((n) => {
            if (n.id === node.id) {
              return { ...n, children: childDetails }
            }
            if (n.children && Array.isArray(n.children)) {
              return { ...n, children: updateNodeInTree(n.children as NodeType[]) }
            }
            return n
          })
        }

        setNodes((prev) => updateNodeInTree(prev))
      } catch (error) {
        console.error('Failed to load children:', error)
      }
    },
    [repo, branch]
  )

  // Select a node and update URL
  const selectNode = useCallback((node: FunctionNode | null) => {
    setSelectedNode(node)
    if (node && repo && branch) {
      navigate(`/${repo}/functions/${branch}${node.path}`)
    }
  }, [repo, branch, navigate])

  // Expand a node
  const expandNode = useCallback((nodeId: string) => {
    setExpandedNodes((prev) => new Set(prev).add(nodeId))
  }, [])

  // Collapse a node
  const collapseNode = useCallback((nodeId: string) => {
    setExpandedNodes((prev) => {
      const next = new Set(prev)
      next.delete(nodeId)
      return next
    })
  }, [])

  // Open a tab for a node (Asset file or Function)
  const openTab = useCallback((node: NodeType) => {
    setOpenTabs((prev) => {
      // Check if tab already exists
      const existing = prev.find((t) => t.path === node.path || t.id === node.id)
      if (existing) {
        setActiveTabId(existing.id)
        return prev
      }

      // Determine language from file extension or node properties
      let language: FunctionLanguage = 'javascript'
      if (node.node_type === 'raisin:Asset') {
        const ext = node.name.split('.').pop()?.toLowerCase()
        if (ext === 'ts') language = 'javascript' // TypeScript uses JS editor
        else if (ext === 'sql') language = 'sql'
        else if (ext === 'py' || ext === 'star' || ext === 'bzl') language = 'starlark'
      } else {
        language = (node.properties?.language as FunctionLanguage) || 'javascript'
      }

      // Create new tab
      const newTab: EditorTab = {
        id: node.id,
        path: node.path,
        name: node.name,
        node_type: node.node_type,
        language,
        isDirty: false,
      }

      setActiveTabId(newTab.id)
      preferencesHook.addOpenTab(node.path)

      return [...prev, newTab]
    })
  }, [preferencesHook])

  // Close a tab
  const closeTab = useCallback((tabId: string) => {
    setOpenTabs((prev) => {
      const tab = prev.find((t) => t.id === tabId)
      if (tab) {
        preferencesHook.removeOpenTab(tab.path)
      }

      const idx = prev.findIndex((t) => t.id === tabId)
      const next = prev.filter((t) => t.id !== tabId)

      // If closing active tab, select adjacent tab
      if (activeTabId === tabId && next.length > 0) {
        const newIdx = Math.min(idx, next.length - 1)
        setActiveTabId(next[newIdx].id)
      } else if (next.length === 0) {
        setActiveTabId(null)
      }

      return next
    })
  }, [activeTabId, preferencesHook])

  // Set active tab
  const setActiveTabHandler = useCallback((tabId: string) => {
    setActiveTabId(tabId)
  }, [])

  // Mark tab as dirty/clean
  const markTabDirty = useCallback((tabId: string, isDirty: boolean) => {
    setOpenTabs((prev) =>
      prev.map((t) => (t.id === tabId ? { ...t, isDirty } : t))
    )
  }, [])

  // Update tab's path (used when we discover the actual file path)
  const updateTabPath = useCallback((tabId: string, newPath: string) => {
    setOpenTabs((prev) =>
      prev.map((t) => (t.id === tabId ? { ...t, path: newPath } : t))
    )
  }, [])

  // Get code from cache
  const getCode = useCallback((path: string): string | undefined => {
    return codeCache.get(path)
  }, [codeCache])

  // Set code in cache
  const setCode = useCallback((path: string, code: string) => {
    codeCache.set(path, code)
  }, [codeCache])

  // Load code for a node (Asset or Function)
  // For Asset: loads properties.file directly
  // For Function: finds entry file and loads it (legacy behavior)
  // Returns { code, filePath } where filePath is the actual file node's path
  const loadCode = useCallback(
    async (node: NodeType): Promise<{ code: string; filePath: string } | null> => {
      if (!repo || !branch) return null

      try {
        // If this is an Asset node, load its content directly
        if (node.node_type === 'raisin:Asset') {
          // Check cache first
          const cached = codeCache.get(node.path)
          if (cached !== undefined) {
            return { code: cached, filePath: node.path }
          }

          // Download the file content directly
          try {
            const code = await nodesApi.downloadFile(
              repo,
              branch,
              FUNCTIONS_WORKSPACE,
              node.path,
              'file',
              true
            ) as string

            codeCache.set(node.path, code)
            return { code, filePath: node.path }
          } catch (downloadError) {
            // File node exists but properties.file is empty
            console.log(`File "${node.name}" exists but has no content yet`)
            codeCache.set(node.path, '')
            return { code: '', filePath: node.path }
          }
        }

        // For Function nodes: find entry file and load it
        // Get entry_file from function properties (format: "filename:function" e.g., "index.js:handler")
        const entryFile = (node.properties?.entry_file as string) ||
          (node.properties?.entrypoint as string) ||
          'index.js:handler'

        // Parse the file name from entry_file
        const fileName = entryFile.includes(':')
          ? entryFile.split(':')[0]
          : 'index.js'

        // Load the function's children to find the file
        const children = await nodesApi.listChildrenAtHead(repo, branch, FUNCTIONS_WORKSPACE, node.path)

        // Update the node tree with the children so file tree is visible in explorer
        const updateNodeInTree = (nodes: NodeType[]): NodeType[] => {
          return nodes.map((n) => {
            if (n.id === node.id) {
              return { ...n, children }
            }
            if (n.children && Array.isArray(n.children)) {
              return { ...n, children: updateNodeInTree(n.children as NodeType[]) }
            }
            return n
          })
        }
        setNodes((prev) => updateNodeInTree(prev))

        // Find the entry file node by name (path may have normalized characters)
        const fileNode = children.find(
          (child) =>
            child.node_type === 'raisin:Asset' && child.name === fileName
        )

        if (!fileNode) {
          // No entry file exists yet - return null to trigger the empty state
          console.log(`Entry file "${fileName}" not found for function ${node.name}`)
          return null
        }

        // Check cache first (keyed by file path)
        const cached = codeCache.get(fileNode.path)
        if (cached !== undefined) {
          return { code: cached, filePath: fileNode.path }
        }

        // Download the file content from the Asset node's file property
        try {
          const code = await nodesApi.downloadFile(
            repo,
            branch,
            FUNCTIONS_WORKSPACE,
            fileNode.path,
            'file',
            true
          ) as string

          codeCache.set(fileNode.path, code)
          return { code, filePath: fileNode.path }
        } catch (downloadError) {
          // File node exists but properties.file is empty - return empty string to allow editing
          console.log(`File "${fileNode.name}" exists but has no content yet`)
          codeCache.set(fileNode.path, '')
          return { code: '', filePath: fileNode.path }
        }
      } catch (error) {
        console.error('Failed to load code:', error)
        return null
      }
    },
    [repo, branch, codeCache, setNodes]
  )

  // Log actions
  const addLog = useCallback((log: LogEntry) => {
    setLogs((prev) => [...prev, log])
  }, [])

  const clearLogs = useCallback(() => {
    setLogs([])
  }, [])

  // Execution actions
  const addExecution = useCallback((execution: ExecutionRecord) => {
    setExecutions((prev) => [execution, ...prev])
  }, [])

  const updateExecution = useCallback((executionId: string, update: Partial<ExecutionRecord>) => {
    setExecutions((prev) =>
      prev.map((e) => (e.execution_id === executionId ? { ...e, ...update } : e))
    )
  }, [])

  const clearExecutions = useCallback(() => {
    setExecutions([])
  }, [])

  // Problems actions
  const setProblems = useCallback((newProblems: ValidationProblem[]) => {
    setProblemsState(newProblems)
  }, [])

  const clearProblems = useCallback(() => {
    setProblemsState([])
  }, [])

  // Tree manipulation actions
  const renameNode = useCallback(
    async (node: NodeType, newName: string, commit: CommitInfo) => {
      if (!repo || !branch) return

      await nodesApi.rename(repo, branch, FUNCTIONS_WORKSPACE, node.path, {
        newName,
        commit: { message: commit.message, actor: commit.actor },
      })

      // Refresh the tree
      await loadRootNodes()
      setRenamingNodeId(null)
    },
    [repo, branch, loadRootNodes]
  )

  const deleteNode = useCallback(
    async (node: NodeType, commit: CommitInfo) => {
      if (!repo || !branch) return

      await nodesApi.delete(repo, branch, FUNCTIONS_WORKSPACE, node.path, {
        commit: { message: commit.message, actor: commit.actor },
      })

      // Refresh the tree
      await loadRootNodes()

      // Clear selection if deleted node was selected
      if (selectedNode?.id === node.id) {
        setSelectedNode(null)
      }

      // Close tab if function was open
      setOpenTabs((prev) => prev.filter((t) => t.path !== node.path))
    },
    [repo, branch, loadRootNodes, selectedNode]
  )

  const moveNode = useCallback(
    async (node: NodeType, destinationPath: string, commit: CommitInfo) => {
      if (!repo || !branch) return

      await nodesApi.move(repo, branch, FUNCTIONS_WORKSPACE, node.path, {
        destination: destinationPath,
        commit: { message: commit.message, actor: commit.actor },
      })

      // Refresh the tree
      await loadRootNodes()
    },
    [repo, branch, loadRootNodes]
  )

  const reorderNode = useCallback(
    async (node: NodeType, targetPath: string, position: 'before' | 'after', commit: CommitInfo) => {
      if (!repo || !branch) return

      await nodesApi.reorder(repo, branch, FUNCTIONS_WORKSPACE, node.path, {
        targetPath,
        position,
        commit: { message: commit.message, actor: commit.actor },
      })

      // Refresh the tree
      await loadRootNodes()
    },
    [repo, branch, loadRootNodes]
  )

  // Check if agents are allowed in the workspace (ai-tools package installed)
  const canCreateAgents = workspaceAllowedTypes.includes('raisin:AIAgent')

  // Helper to determine what can be created in a parent
  const getCreationOptions = useCallback(
    (parentNode: NodeType | null): { canCreateFolder: boolean; canCreateFunction: boolean; canCreateFile: boolean; canCreateAgent: boolean } => {
      // Check if we're in the agents folder
      const isAgentsFolder = parentNode?.path === '/agents' || parentNode?.name === 'agents'

      // Root level: only folders allowed (and agents in /agents folder)
      if (!parentNode) {
        return { canCreateFolder: true, canCreateFunction: false, canCreateFile: false, canCreateAgent: false }
      }

      // Inside the agents folder: can create agents
      if (isAgentsFolder && canCreateAgents) {
        return { canCreateFolder: true, canCreateFunction: false, canCreateFile: false, canCreateAgent: true }
      }

      // Inside a function: can create files and folders
      if (parentNode.node_type === 'raisin:Function') {
        return { canCreateFolder: true, canCreateFunction: false, canCreateFile: true, canCreateAgent: false }
      }

      // Get children of the parent
      const children = parentNode.children as NodeType[] | undefined
      if (!children || children.length === 0) {
        // Empty folder: can create either functions or folders
        return { canCreateFolder: true, canCreateFunction: true, canCreateFile: false, canCreateAgent: false }
      }

      // Check what types exist
      const hasSubfolders = children.some((c) => c.node_type === 'raisin:Folder')
      const hasFunctions = children.some((c) => c.node_type === 'raisin:Function')
      const hasAssets = children.some((c) => c.node_type === 'raisin:Asset')
      const hasAgents = children.some((c) => c.node_type === 'raisin:AIAgent')

      // If folder contains agents, allow creating more agents
      if (hasAgents && canCreateAgents) {
        return { canCreateFolder: false, canCreateFunction: false, canCreateFile: false, canCreateAgent: true }
      }

      // If folder contains assets, it's likely inside a function
      if (hasAssets) {
        return { canCreateFolder: true, canCreateFunction: false, canCreateFile: true, canCreateAgent: false }
      }

      if (hasSubfolders) {
        // Has subfolders: can only add more folders
        return { canCreateFolder: true, canCreateFunction: false, canCreateFile: false, canCreateAgent: false }
      }

      if (hasFunctions) {
        // Has functions: can only add more functions (leaf folder)
        return { canCreateFolder: false, canCreateFunction: true, canCreateFile: false, canCreateAgent: false }
      }

      // Fallback
      return { canCreateFolder: true, canCreateFunction: true, canCreateFile: false, canCreateAgent: false }
    },
    [canCreateAgents]
  )

  // Load executions from server for selected function
  const loadExecutions = useCallback(async () => {
    if (!repo || !selectedNode) return

    const functionName = (selectedNode as FunctionNode).properties?.name as string || selectedNode.name

    setExecutionsLoading(true)
    try {
      const executionRecords = await functionsApi.listExecutions(repo, functionName, { limit: 50 })

      // Map API response to our ExecutionRecord type
      const mapped: ExecutionRecord[] = executionRecords.map((record) => ({
        id: record.execution_id,
        execution_id: record.execution_id,
        function_path: record.function_path,
        trigger_name: record.trigger_name,
        status: record.status as ExecutionRecord['status'],
        started_at: record.started_at,
        completed_at: record.completed_at,
        duration_ms: record.duration_ms,
        result: record.result,
        error: record.error,
        logs: [],
      }))

      setExecutions(mapped)
    } catch (error) {
      console.error('Failed to load executions:', error)
      // Don't clear existing executions on error
    } finally {
      setExecutionsLoading(false)
    }
  }, [repo, selectedNode])

  // Load workspace definition to check allowed node types
  useEffect(() => {
    async function loadWorkspaceDefinition() {
      if (!repo) return
      try {
        const ws = await workspacesApi.get(repo, FUNCTIONS_WORKSPACE)
        setWorkspaceAllowedTypes(ws.allowed_node_types || [])
      } catch (error) {
        console.error('Failed to load workspace definition:', error)
      }
    }
    loadWorkspaceDefinition()
  }, [repo])

  // Load root nodes on mount
  useEffect(() => {
    console.log("useEffect: Loading root function nodes on mount or repo/branch change")
    loadRootNodes()
  }, [loadRootNodes])

  // Load executions when selected node changes
  useEffect(() => {
    console.log("useEffect: Loading executions when selected node changes")
    if (selectedNode && selectedNode.node_type === 'raisin:Function') {
      loadExecutions()
    } else {
      // Clear executions when no function is selected
      setExecutions([])
    }
  }, [selectedNode, loadExecutions])

  // Sync expanded nodes to preferences
  useEffect(() => {
    console.log("useEffect: Syncing expanded nodes to preferences")
    setExpandedFolders(Array.from(expandedNodes))
  }, [expandedNodes, setExpandedFolders])

  // Handle URL path changes - expand tree and select node
  useEffect(() => {
    console.log("useEffect: Handling URL path changes to expand tree and select node")
    // Guard: only run if we're actually on a functions route
    if (!location.pathname.includes('/functions/')) return
    if (!normalizedUrlPath || loading || nodes.length === 0 || !repo || !branch) return

    let cancelled = false

    async function selectNodeFromPath() {
      const dedupedPath = normalizedUrlPath!.replace(/(\.[^/.]+)(\1)+(\/?)$/, '$1$3')
      try {
        console.log("Selecting node from URL path", dedupedPath)
        const fetchPath = (path: string) => nodesApi.getAtHead(repo!, branch!, FUNCTIONS_WORKSPACE, path)

        let targetNode = await fetchPath(dedupedPath)
        if (cancelled || !targetNode) return

        // Expand all parent folders
        const pathParts = dedupedPath.split('/').filter(Boolean)
        const parentPaths: string[] = []
        for (let i = 0; i < pathParts.length - 1; i++) {
          parentPaths.push('/' + pathParts.slice(0, i + 1).join('/'))
        }

        for (const parentPath of parentPaths) {
          if (cancelled) return
          try {
            const parentNode = await nodesApi.getAtHead(repo!, branch!, FUNCTIONS_WORKSPACE, parentPath)
            if (cancelled) return
            if (parentNode) {
              // Load children to populate the tree and expand the node
              await loadNodeChildren(parentNode)
              if (cancelled) return
              setExpandedNodes(prev => new Set(prev).add(parentNode.id))
            }
          } catch (e) {
            if (!cancelled) {
              console.error(`Failed to load parent node ${parentPath}:`, e)
            }
          }
        }

        if (!cancelled) {
          setSelectedNode(targetNode as unknown as FunctionNode)
        }
      } catch (error) {
        if (cancelled) return
        console.error('Failed to select node from URL path:', dedupedPath, error)
      }
    }

    selectNodeFromPath()

    return () => { cancelled = true }
  }, [normalizedUrlPath, loading, nodes.length, repo, branch, location.pathname, loadNodeChildren])

  // If the wildcard path was normalized (e.g., to remove duplicate extensions), replace the URL to the cleaned version
  useEffect(() => {
    if (!rawWildcardPath || !urlPath || rawWildcardPath === urlPath) return
    if (!location.pathname.includes('/functions/')) return
    if (!repo || !branch) return
    navigate(`/${repo}/functions/${branch}${urlPath}`, { replace: true })
  }, [rawWildcardPath, urlPath, repo, branch, location.pathname, navigate])

  const value: FunctionsContextValue = {
    // Repository context
    repo: repo || '',
    branch: branch || '',
    workspace: FUNCTIONS_WORKSPACE,

    // Tree state
    nodes,
    expandedNodes,
    selectedNode,
    loading,

    // Editor state
    openTabs,
    activeTabId,
    codeCache,

    // Output state
    logs,
    executions,
    executionsLoading,
    problems,

    // Rename state
    renamingNodeId,

    // Tree actions
    loadRootNodes,
    loadNodeChildren,
    selectNode,
    expandNode,
    collapseNode,

    // Rename actions
    setRenamingNodeId,
    renameNode,
    deleteNode,
    moveNode,
    reorderNode,

    // Creation rules
    getCreationOptions,

    // Tab actions
    openTab,
    closeTab,
    setActiveTab: setActiveTabHandler,
    markTabDirty,
    updateTabPath,

    // Code actions
    getCode,
    setCode,
    loadCode,

    // Log actions
    addLog,
    clearLogs,
    addExecution,
    updateExecution,
    clearExecutions,
    loadExecutions,

    // Problems actions
    setProblems,
    clearProblems,

    // Preferences
    ...preferencesHook,
  }

  return (
    <FunctionsContext.Provider value={value}>
      {children}
    </FunctionsContext.Provider>
  )
}

export function useFunctionsContext() {
  const context = useContext(FunctionsContext)
  if (!context) {
    throw new Error('useFunctionsContext must be used within a FunctionsProvider')
  }
  return context
}
