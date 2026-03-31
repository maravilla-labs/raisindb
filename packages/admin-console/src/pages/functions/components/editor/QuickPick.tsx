/**
 * QuickPick Component
 *
 * VS Code Command Palette-style modal for selecting nodes.
 * Features search-first design with keyboard navigation.
 */

import { useState, useEffect, useCallback, useRef, useMemo } from 'react'
import { Search, X, Folder, FileText, Database, Clock, ChevronRight, ChevronDown, Loader2 } from 'lucide-react'
import { nodesApi, type Node } from '../../../../api/nodes'
import { workspacesApi } from '../../../../api/workspaces'
import { useRepositoryContext } from '../../../../hooks/useRepositoryContext'

interface QuickPickProps {
  /** Called when a node is selected */
  onSelect: (nodeId: string, nodePath: string, workspace: string) => void
  /** Called when the picker is closed */
  onClose: () => void
  /** Initial workspace to show */
  initialWorkspace?: string
}

interface RecentNode {
  id: string
  path: string
  workspace: string
  name: string
  nodeType: string
}

// Load recent nodes from localStorage
function loadRecentNodes(): RecentNode[] {
  try {
    const saved = localStorage.getItem('raisindb.functions.recentInputNodes')
    return saved ? JSON.parse(saved) : []
  } catch {
    return []
  }
}

// Save recent nodes to localStorage
function saveRecentNodes(nodes: RecentNode[]) {
  try {
    // Keep only last 5
    const toSave = nodes.slice(0, 5)
    localStorage.setItem('raisindb.functions.recentInputNodes', JSON.stringify(toSave))
  } catch {
    // Ignore storage errors
  }
}

// Add a node to recent list
export function addToRecentNodes(node: RecentNode) {
  const recent = loadRecentNodes()
  // Remove if already exists
  const filtered = recent.filter(n => n.id !== node.id)
  // Add to front
  const updated = [node, ...filtered].slice(0, 5)
  saveRecentNodes(updated)
}

export function QuickPick({ onSelect, onClose, initialWorkspace = 'content' }: QuickPickProps) {
  const { repo, branch } = useRepositoryContext()
  const [search, setSearch] = useState('')
  const [selectedWorkspace, setSelectedWorkspace] = useState(initialWorkspace)
  const [workspaces, setWorkspaces] = useState<string[]>(['functions', 'content', 'system'])
  const [nodes, setNodes] = useState<Node[]>([])
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [childrenCache, setChildrenCache] = useState<Map<string, Node[]>>(new Map())
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(true)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [recentNodes] = useState<RecentNode[]>(() => loadRecentNodes())

  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  // Load nodes when workspace changes
  useEffect(() => {
    const loadNodes = async () => {
      if (!repo || !branch) return
      setLoading(true)
      try {
        const data = await nodesApi.listRootAtHead(repo, branch, selectedWorkspace)
        setNodes(data)
        setExpandedPaths(new Set())
        setChildrenCache(new Map())
      } catch (error) {
        console.error('Failed to load nodes:', error)
        setNodes([])
      } finally {
        setLoading(false)
      }
    }
    loadNodes()
  }, [repo, branch, selectedWorkspace])

  // Load workspaces list
  useEffect(() => {
    const fetchWorkspaces = async () => {
      if (!repo) return
      try {
        const list = await workspacesApi.list(repo)
        const names = list.map(ws => ws.name)
        if (names.length > 0) {
          setWorkspaces(names)
          if (!names.includes(selectedWorkspace)) {
            setSelectedWorkspace(initialWorkspace && names.includes(initialWorkspace) ? initialWorkspace : names[0])
          }
          return
        }
      } catch (error) {
        console.error('Failed to load workspaces for QuickPick:', error)
      }
      // Fallback if API fails or returns empty
      setWorkspaces(prev => {
        const defaults = ['functions', 'content', 'system']
        return prev.length ? prev : defaults
      })
      setSelectedWorkspace(prev => prev || initialWorkspace || 'content')
    }
    fetchWorkspaces()
  }, [repo, initialWorkspace, selectedWorkspace])

  // Reset selection when search changes
  useEffect(() => {
    setSelectedIndex(0)
  }, [search])

  useEffect(() => {
    setSelectedIndex(0)
  }, [selectedWorkspace])

  // Filter nodes by search
  const filteredNodes = useMemo(() => {
    if (!search) return nodes
    const lowerSearch = search.toLowerCase()
    return nodes.filter(n =>
      n.name.toLowerCase().includes(lowerSearch) ||
      n.path.toLowerCase().includes(lowerSearch)
    )
  }, [nodes, search])

  // Filter recent nodes by search
  const filteredRecent = useMemo(() => {
    if (!search) return recentNodes
    const lowerSearch = search.toLowerCase()
    return recentNodes.filter(n =>
      n.name.toLowerCase().includes(lowerSearch) ||
      n.path.toLowerCase().includes(lowerSearch)
    )
  }, [recentNodes, search])

  // All items (recent + current workspace)
  const allItems = useMemo(() => {
    const items: Array<{ type: 'recent' | 'node'; data: RecentNode | Node }> = []

    // Add recent items first
    filteredRecent.forEach(n => items.push({ type: 'recent', data: n }))

    // Add workspace nodes
    filteredNodes.forEach(n => items.push({ type: 'node', data: n }))

    return items
  }, [filteredRecent, filteredNodes])

  // Handle keyboard navigation
  // Build tree items for browsing
  const treeItems = useMemo(() => {
    const items: Array<{ key: string; node: Node; depth: number }> = []
    const walk = (list: Node[], depth: number) => {
      list.forEach((node) => {
        const key = `node-${node.id}`
        items.push({ key, node, depth })
        const isExpanded = expandedPaths.has(node.path)
        if (isExpanded) {
          const children = childrenCache.get(node.path) || []
          walk(children, depth + 1)
        }
      })
    }
    walk(nodes, 0)
    return items
  }, [nodes, expandedPaths, childrenCache])

  const isSearchMode = search.trim().length > 0

  const visibleItems = useMemo(() => {
    if (isSearchMode) {
      return allItems.map((item, idx) => ({
        key: `${item.type}-${idx}-${'id' in item.data ? (item.data as any).id : idx}`,
        type: item.type as 'recent' | 'node',
        workspace: item.type === 'recent' ? (item.data as RecentNode).workspace : selectedWorkspace,
        node: item.type === 'node' ? item.data as Node : null,
        recent: item.type === 'recent' ? item.data as RecentNode : null,
        depth: 0,
      }))
    }

    const items: Array<{
      key: string
      type: 'recent' | 'node'
      workspace: string
      node: Node | null
      recent: RecentNode | null
      depth: number
    }> = []

    filteredRecent.forEach((rec) => {
      items.push({
        key: `recent-${rec.id}`,
        type: 'recent',
        workspace: rec.workspace,
        node: null,
        recent: rec,
        depth: 0,
      })
    })

    treeItems.forEach((entry) => {
      items.push({
        key: entry.key,
        type: 'node',
        workspace: selectedWorkspace,
        node: entry.node,
        recent: null,
        depth: entry.depth,
      })
    })

    return items
  }, [isSearchMode, allItems, filteredRecent, treeItems, selectedWorkspace])

  const selectItem = useCallback((itemIdx: number) => {
    const item = visibleItems[itemIdx]
    if (!item) return

    if (item.type === 'recent' && item.recent) {
      addToRecentNodes(item.recent)
      onSelect(item.recent.id, `${item.recent.workspace}:${item.recent.path}`, item.recent.workspace)
      return
    }

    if (item.node) {
      addToRecentNodes({
        id: item.node.id,
        path: item.node.path,
        workspace: item.workspace,
        name: item.node.name,
        nodeType: item.node.node_type,
      })
      onSelect(item.node.id, `${item.workspace}:${item.node.path}`, item.workspace)
    }
  }, [visibleItems, onSelect])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        setSelectedIndex(i => Math.min(i + 1, visibleItems.length - 1))
        break
      case 'ArrowUp':
        e.preventDefault()
        setSelectedIndex(i => Math.max(i - 1, 0))
        break
      case 'Enter':
        e.preventDefault()
        selectItem(selectedIndex)
        break
      case 'Escape':
        e.preventDefault()
        onClose()
        break
    }
  }, [visibleItems.length, selectedIndex, selectItem, onClose])

  useEffect(() => {
    setSelectedIndex((idx) => Math.min(idx, Math.max(visibleItems.length - 1, 0)))
  }, [visibleItems.length])

  // Scroll selected item into view
  useEffect(() => {
    if (listRef.current) {
      const selected = listRef.current.querySelector(`[data-index="${selectedIndex}"]`)
      selected?.scrollIntoView({ block: 'nearest' })
    }
  }, [selectedIndex])

  const renderItem = (item: {
    key: string
    type: 'recent' | 'node'
    workspace: string
    node: Node | null
    recent: RecentNode | null
    depth: number
  }, idx: number) => {
    const isSelected = selectedIndex === idx
    const isFolder = item.node?.node_type.includes('Folder')
    const hasChildren = isFolder && (item.node?.has_children || (item.node && (childrenCache.get(item.node.path)?.length ?? 0) > 0))
    const isExpanded = isFolder && item.node ? expandedPaths.has(item.node.path) : false
    const isLoadingPath = isFolder && item.node ? loadingPaths.has(item.node.path) : false

    const displayName = item.node
      ? item.node.name
      : item.recent
        ? item.recent.name
        : ''
    const displayPath = item.node
      ? item.node.path
      : item.recent
        ? `${item.recent.workspace}:${item.recent.path}`
        : ''

    const toggleFolder = (e: React.MouseEvent) => {
      e.stopPropagation()
      if (!item.node || !repo || !branch) return
      const path = item.node.path
      const isExpandedNow = expandedPaths.has(path)
      if (isExpandedNow) {
        setExpandedPaths(prev => {
          const next = new Set(prev)
          next.delete(path)
          return next
        })
        return
      }

      if (!childrenCache.has(path)) {
        setLoadingPaths(prev => new Set(prev).add(path))
        nodesApi.listChildrenAtHead(repo, branch, item.workspace, path)
          .then(children => {
            setChildrenCache(prev => new Map(prev).set(path, children))
          })
          .catch(error => {
            console.error('Failed to load children:', error)
          })
          .finally(() => {
            setLoadingPaths(prev => {
              const next = new Set(prev)
              next.delete(path)
              return next
            })
          })
      }
      setExpandedPaths(prev => new Set(prev).add(path))
    }

    return (
      <button
        key={item.key}
        data-index={idx}
        onClick={() => selectItem(idx)}
        className={`w-full px-4 py-2 flex items-center gap-3 text-left transition-colors ${
          isSelected ? 'bg-primary-500/20 text-white' : 'text-gray-300 hover:bg-white/5'
        }`}
        style={{ paddingLeft: item.type === 'node' ? `${12 + item.depth * 16}px` : undefined }}
      >
        {/* Chevron / spacer */}
        {isFolder ? (
          <button
            onClick={toggleFolder}
            className="w-4 h-4 flex items-center justify-center flex-shrink-0 text-gray-500 hover:text-white"
            title={isExpanded ? 'Collapse' : 'Expand'}
          >
            {isLoadingPath ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : isExpanded ? (
              <ChevronDown className="w-4 h-4" />
            ) : hasChildren ? (
              <ChevronRight className="w-4 h-4" />
            ) : (
              <span className="w-4 h-4" />
            )}
          </button>
        ) : item.type === 'node' ? (
          <span className="w-4" />
        ) : (
          <Clock className="w-4 h-4 text-gray-500 flex-shrink-0" />
        )}

        {/* Icon */}
        {isFolder ? (
          <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
        ) : (
          <FileText className="w-4 h-4 text-gray-400 flex-shrink-0" />
        )}

        <div className="flex-1 min-w-0">
          <div className="truncate">{displayName}</div>
          <div className="text-xs text-gray-500 truncate">
            {displayPath}
          </div>
        </div>

        {item.node && (
          <span className="text-xs text-gray-500">{item.node.node_type}</span>
        )}
      </button>
    )
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh] bg-black/50"
      onClick={onClose}
    >
      <div
        className="w-[600px] max-h-[60vh] bg-gray-900 rounded-lg shadow-2xl border border-white/10 flex flex-col overflow-hidden"
        onClick={e => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        {/* Search Input */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-white/10">
          <Search className="w-5 h-5 text-gray-400" />
          <input
            ref={inputRef}
            type="text"
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder="Search nodes..."
            className="flex-1 bg-transparent text-white placeholder-gray-500 outline-none text-lg"
          />
          <button
            onClick={onClose}
            className="p-1 text-gray-400 hover:text-white transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Workspace Tabs */}
        <div className="flex border-b border-white/10 px-2 overflow-x-auto">
          {workspaces.map(ws => (
            <button
              key={ws}
              onClick={() => setSelectedWorkspace(ws)}
              className={`px-4 py-2 text-sm transition-colors whitespace-nowrap ${
                selectedWorkspace === ws
                  ? 'text-primary-300 border-b-2 border-primary-500'
                  : 'text-gray-400 hover:text-white'
              }`}
            >
              {ws}
            </button>
          ))}
        </div>

        {/* Results */}
        <div ref={listRef} className="flex-1 overflow-auto">
          {loading ? (
            <div className="p-8 text-center text-gray-400">Loading...</div>
          ) : visibleItems.length === 0 ? (
            <div className="p-8 text-center text-gray-400">
              {search ? 'No results found' : 'No nodes in this workspace'}
            </div>
          ) : (
            <div className="py-2">
              {!isSearchMode && filteredRecent.length > 0 && (
                <div className="px-4 py-1 text-xs text-gray-500 uppercase tracking-wider flex items-center gap-2">
                  <Clock className="w-3 h-3" />
                  Recently Used
                </div>
              )}

              {!isSearchMode && (
                <div className="px-4 py-1 text-xs text-gray-500 uppercase tracking-wider flex items-center gap-2">
                  <Database className="w-3 h-3" />
                  {selectedWorkspace}
                </div>
              )}

              {visibleItems.map((item, idx) => renderItem(item, idx))}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-4 py-2 border-t border-white/10 text-xs text-gray-500 flex items-center gap-4">
          <span><kbd className="px-1 bg-gray-800 rounded">↑↓</kbd> Navigate</span>
          <span><kbd className="px-1 bg-gray-800 rounded">Enter</kbd> Select</span>
          <span><kbd className="px-1 bg-gray-800 rounded">Esc</kbd> Close</span>
        </div>
      </div>
    </div>
  )
}
