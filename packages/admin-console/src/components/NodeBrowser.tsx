import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import {
  ChevronRight,
  ChevronDown,
  Folder,
  FileText,
  X,
  Loader2,
  Home,
  Search
} from 'lucide-react'
import { nodesApi, type Node } from '../api/nodes'
import { workspacesApi, type Workspace } from '../api/workspaces'

interface NodeBrowserProps {
  repo: string
  branch: string
  currentWorkspace: string
  onSelect: (workspace: string, path: string, nodeName: string, nodeType: string) => void
  onClose: () => void
  excludePath?: string // Don't allow selecting this path (to prevent circular references)
}

export function NodeBrowser({
  repo,
  branch,
  currentWorkspace,
  onSelect,
  onClose,
  excludePath
}: NodeBrowserProps) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [selectedWorkspace, setSelectedWorkspace] = useState(currentWorkspace)
  const [nodes, setNodes] = useState<Node[]>([])
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(true)
  const [loadingChildren, setLoadingChildren] = useState<Set<string>>(new Set())
  const [searchTerm, setSearchTerm] = useState('')
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const modalRef = useRef<HTMLDivElement>(null)

  // Load workspaces on mount
  useEffect(() => {
    loadWorkspaces()
  }, [repo])

  // Load nodes when workspace changes
  useEffect(() => {
    if (selectedWorkspace) {
      loadRootNodes()
    }
  }, [selectedWorkspace, repo, branch])

  // Handle escape key
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleEscape)
    modalRef.current?.focus()
    return () => document.removeEventListener('keydown', handleEscape)
  }, [onClose])

  async function loadWorkspaces() {
    try {
      const data = await workspacesApi.list(repo)
      setWorkspaces(data)
    } catch (error) {
      console.error('Failed to load workspaces:', error)
    }
  }

  async function loadRootNodes() {
    try {
      setLoading(true)
      const data = await nodesApi.listRootAtHead(repo, branch, selectedWorkspace)
      setNodes(data)
    } catch (error) {
      console.error('Failed to load root nodes:', error)
    } finally {
      setLoading(false)
    }
  }

  async function loadNodeChildren(node: Node) {
    // If children already loaded, just toggle expansion
    if (node.children && node.children.length > 0 && typeof node.children[0] === 'object') {
      toggleNodeExpansion(node.id)
      return
    }

    try {
      setLoadingChildren(prev => new Set(prev).add(node.id))
      const childDetails = await nodesApi.listChildrenAtHead(repo, branch, selectedWorkspace, node.path)

      // Update the nodes tree with children
      const updateNodeInTree = (nodes: Node[]): Node[] => {
        return nodes.map(n => {
          if (n.id === node.id) {
            return { ...n, children: childDetails }
          }
          if (n.children && Array.isArray(n.children)) {
            return { ...n, children: updateNodeInTree(n.children as Node[]) }
          }
          return n
        })
      }

      setNodes(updateNodeInTree(nodes))
      setExpandedNodes(prev => new Set(prev).add(node.id))
    } catch (error) {
      console.error('Failed to load children:', error)
    } finally {
      setLoadingChildren(prev => {
        const next = new Set(prev)
        next.delete(node.id)
        return next
      })
    }
  }

  function toggleNodeExpansion(nodeId: string) {
    setExpandedNodes(prev => {
      const next = new Set(prev)
      if (next.has(nodeId)) {
        next.delete(nodeId)
      } else {
        next.add(nodeId)
      }
      return next
    })
  }

  function handleNodeClick(node: Node) {
    // Don't allow selecting the excluded path
    if (excludePath && node.path === excludePath) {
      return
    }
    setSelectedPath(node.path)
  }

  function handleNodeExpand(node: Node, e: React.MouseEvent) {
    e.stopPropagation()
    if (expandedNodes.has(node.id)) {
      toggleNodeExpansion(node.id)
    } else {
      loadNodeChildren(node)
    }
  }

  function handleSelect() {
    if (selectedPath) {
      const selectedNode = findNodeByPath(nodes, selectedPath)
      if (selectedNode) {
        onSelect(selectedWorkspace, selectedPath, selectedNode.name, selectedNode.node_type)
      }
    }
  }

  function findNodeByPath(nodeList: Node[], path: string): Node | null {
    for (const node of nodeList) {
      if (node.path === path) return node
      if (node.children && Array.isArray(node.children)) {
        const found = findNodeByPath(node.children as Node[], path)
        if (found) return found
      }
    }
    return null
  }

  function filterNodes(nodeList: Node[]): Node[] {
    if (!searchTerm.trim()) return nodeList

    const lowerSearch = searchTerm.toLowerCase()
    return nodeList.filter(node => {
      const matchesName = node.name.toLowerCase().includes(lowerSearch)
      const matchesPath = node.path.toLowerCase().includes(lowerSearch)
      const hasMatchingChildren = node.children && Array.isArray(node.children)
        ? filterNodes(node.children as Node[]).length > 0
        : false

      return matchesName || matchesPath || hasMatchingChildren
    }).map(node => {
      if (node.children && Array.isArray(node.children)) {
        return { ...node, children: filterNodes(node.children as Node[]) }
      }
      return node
    })
  }

  function renderNode(node: Node, depth: number = 0) {
    const hasChildren = node.has_children || (node.children && node.children.length > 0)
    const isExpanded = expandedNodes.has(node.id)
    const isSelected = selectedPath === node.path
    const isExcluded = excludePath === node.path
    const isLoadingThis = loadingChildren.has(node.id)

    return (
      <div key={node.id}>
        <div
          className={`flex items-center gap-2 px-3 py-2 rounded-lg cursor-pointer transition-all
            ${isSelected ? 'bg-primary-500/30 border border-primary-400/50' : 'hover:bg-white/5'}
            ${isExcluded ? 'opacity-50 cursor-not-allowed' : ''}
          `}
          style={{ paddingLeft: `${depth * 1.5 + 0.75}rem` }}
          onClick={() => !isExcluded && handleNodeClick(node)}
        >
          {/* Expand/collapse button */}
          {hasChildren && (
            <button
              onClick={(e) => handleNodeExpand(node, e)}
              className="p-0.5 hover:bg-white/10 rounded transition-colors"
              disabled={isLoadingThis}
              aria-label={isExpanded ? 'Collapse' : 'Expand'}
            >
              {isLoadingThis ? (
                <Loader2 className="w-4 h-4 animate-spin text-primary-400" />
              ) : isExpanded ? (
                <ChevronDown className="w-4 h-4 text-zinc-400" />
              ) : (
                <ChevronRight className="w-4 h-4 text-zinc-400" />
              )}
            </button>
          )}
          {!hasChildren && <div className="w-5" />}

          {/* Node icon */}
          {hasChildren ? (
            <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
          ) : (
            <FileText className="w-4 h-4 text-zinc-400 flex-shrink-0" />
          )}

          {/* Node name and type */}
          <div className="flex-1 min-w-0">
            <div className="text-sm text-white truncate">{node.name}</div>
            <div className="text-xs text-zinc-500 truncate">{node.node_type}</div>
          </div>

          {isExcluded && (
            <div className="text-xs px-2 py-0.5 bg-red-500/20 text-red-400 rounded">
              Current
            </div>
          )}
        </div>

        {/* Children */}
        {isExpanded && node.children && Array.isArray(node.children) && (
          <div>
            {(node.children as Node[]).map(child => renderNode(child, depth + 1))}
          </div>
        )}
      </div>
    )
  }

  const filteredNodes = filterNodes(nodes)

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="node-browser-title"
    >
      <div
        ref={modalRef}
        tabIndex={-1}
        className="bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black backdrop-blur-xl
                   border border-white/20 shadow-2xl rounded-2xl max-w-3xl w-full max-h-[80vh]
                   flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div>
            <h2 id="node-browser-title" className="text-xl font-bold text-white">
              Select Target Node
            </h2>
            <p className="text-sm text-zinc-400 mt-1">
              Choose a workspace and node to create a relationship
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
            aria-label="Close"
          >
            <X className="w-5 h-5 text-zinc-400" />
          </button>
        </div>

        {/* Workspace Selector */}
        <div className="p-6 border-b border-white/10 space-y-3">
          <label className="block text-sm font-medium text-zinc-300">
            Workspace
          </label>
          <select
            value={selectedWorkspace}
            onChange={(e) => {
              setSelectedWorkspace(e.target.value)
              setSelectedPath(null)
              setExpandedNodes(new Set())
            }}
            className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg
                     text-white placeholder-zinc-400
                     focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50
                     transition-all"
            aria-label="Select workspace"
          >
            {workspaces.map((ws) => (
              <option key={ws.name} value={ws.name} className="bg-zinc-900">
                {ws.name}
              </option>
            ))}
          </select>

          {/* Search */}
          <div className="relative">
            <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-zinc-400" />
            <input
              type="text"
              placeholder="Search nodes..."
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              className="w-full pl-10 pr-4 py-2 bg-white/10 border border-white/20 rounded-lg
                       text-white placeholder-zinc-400
                       focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50
                       transition-all"
              aria-label="Search nodes"
            />
          </div>
        </div>

        {/* Node Tree */}
        <div className="flex-1 overflow-auto p-6">
          {loading ? (
            <div className="flex items-center justify-center py-12">
              <Loader2 className="w-6 h-6 animate-spin text-primary-400" />
              <span className="ml-2 text-zinc-400">Loading nodes...</span>
            </div>
          ) : filteredNodes.length === 0 ? (
            <div className="text-center py-12">
              <Home className="w-12 h-12 text-zinc-600 mx-auto mb-3" />
              <p className="text-zinc-500">
                {searchTerm ? 'No nodes match your search' : 'No nodes in this workspace'}
              </p>
            </div>
          ) : (
            <div className="space-y-1">
              {filteredNodes.map(node => renderNode(node))}
            </div>
          )}
        </div>

        {/* Selected Path Display */}
        {selectedPath && (
          <div className="px-6 py-3 bg-white/5 border-t border-white/10">
            <div className="text-xs text-zinc-400 mb-1">Selected Path</div>
            <div className="text-sm text-white font-mono">
              {selectedWorkspace}:{selectedPath}
            </div>
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 p-6 border-t border-white/10">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg
                     transition-colors focus:outline-none focus:ring-2 focus:ring-white/50"
          >
            Cancel
          </button>
          <button
            onClick={handleSelect}
            disabled={!selectedPath}
            className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg
                     transition-all disabled:opacity-50 disabled:cursor-not-allowed
                     focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2
                     focus:ring-offset-zinc-900"
          >
            Select Node
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
