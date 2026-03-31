/**
 * NodePicker Component
 *
 * Reusable modal for selecting nodes from the functions workspace.
 * Supports SQL-based search and tree browsing with expandable folders.
 *
 * Used by FunctionPicker and AgentPicker with different configurations.
 */

import { useState, useEffect, useCallback, useRef, type ComponentType } from 'react'
import { createPortal } from 'react-dom'
import { Search, X, Folder, ChevronRight, ChevronDown, FileText, Loader2 } from 'lucide-react'
import { nodesApi, type Node } from '../api/nodes'
import { sqlApi } from '../api/sql'
import { useRepositoryContext } from '../hooks/useRepositoryContext'

/** Configuration for the NodePicker */
export interface NodePickerConfig {
  /** Node type to select (e.g. 'raisin:Function', 'raisin:AIAgent') */
  nodeType: string

  /** Modal header title */
  title: string

  /** Modal header subtitle */
  subtitle: string

  /** Search input placeholder */
  searchPlaceholder: string

  /** Message when tree is empty */
  emptyMessage: string

  /** Secondary hint for empty state (optional) */
  emptyHint?: string

  /** Icon component for selectable nodes */
  icon: ComponentType<{ className?: string }>

  /** Tailwind color class for icon (e.g. 'text-blue-400') */
  iconColor: string

  /** Selection highlight color (e.g. 'primary-500', 'purple-500') */
  selectionColor: string

  /** Current selection path to highlight (optional) */
  currentPath?: string

  /** Folder name to auto-expand on load (optional) */
  autoExpandFolder?: string

  /** Whether to filter non-matching nodes in tree view (optional) */
  filterTreeNodes?: boolean

  /** Workspace to search in (defaults to 'functions') */
  workspace?: string
}

/** Node data returned on selection */
export interface SelectedNode {
  id: string
  path: string
  name: string
  properties?: Record<string, unknown>
}

export interface NodePickerProps {
  config: NodePickerConfig
  onSelect: (node: SelectedNode) => void
  onClose: () => void
}

interface TreeNode extends Node {
  children?: TreeNode[]
  isLoading?: boolean
  isExpanded?: boolean
}

interface SearchResult {
  id: string
  path: string
  name: string
  title?: string
  description?: string
}

export function NodePicker({ config, onSelect, onClose }: NodePickerProps) {
  const {
    nodeType,
    title,
    subtitle,
    searchPlaceholder,
    emptyMessage,
    emptyHint,
    icon: Icon,
    iconColor,
    selectionColor,
    currentPath,
    autoExpandFolder,
    filterTreeNodes = false,
    workspace = 'functions',
  } = config

  const { repo, branch } = useRepositoryContext()
  const [search, setSearch] = useState('')
  const [treeNodes, setTreeNodes] = useState<TreeNode[]>([])
  const [searchResults, setSearchResults] = useState<SearchResult[]>([])
  const [treeLoading, setTreeLoading] = useState(true)
  const [searchLoading, setSearchLoading] = useState(false)
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(new Set())
  const [childrenCache, setChildrenCache] = useState<Map<string, TreeNode[]>>(new Map())
  const [selectedPath, setSelectedPath] = useState<string | null>(currentPath || null)

  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  // Load root nodes
  useEffect(() => {
    const loadRootNodes = async () => {
      if (!repo || !branch) return
      setTreeLoading(true)
      try {
        const data = await nodesApi.listRootAtHead(repo, branch, workspace)
        setTreeNodes(data)

        // Auto-expand to current path if provided
        if (currentPath) {
          const pathParts = currentPath.split('/')
          const parentPaths: string[] = []

          // Build parent paths (all except the last segment which is the item itself)
          for (let i = 0; i < pathParts.length - 1; i++) {
            parentPaths.push(pathParts.slice(0, i + 1).join('/'))
          }

          if (parentPaths.length > 0) {
            // Load children for each parent path
            const newCache = new Map<string, TreeNode[]>()
            for (const parentPath of parentPaths) {
              try {
                const children = await nodesApi.listChildrenAtHead(repo, branch, workspace, parentPath)
                newCache.set(parentPath, children)
              } catch {
                // Parent might not exist, skip
              }
            }
            setChildrenCache(newCache)
            setExpandedPaths(new Set(parentPaths))
          }
        }
        // Auto-expand specified folder if it exists (fallback when no currentPath)
        else if (autoExpandFolder) {
          const folder = data.find(
            (n) => n.name === autoExpandFolder && n.node_type === 'raisin:Folder'
          )
          if (folder) {
            setExpandedPaths(new Set([folder.path]))
            // Load folder children
            const children = await nodesApi.listChildrenAtHead(repo, branch, workspace, folder.path)
            setChildrenCache(new Map([[folder.path, children]]))
          }
        }
      } catch (error) {
        console.error('Failed to load nodes:', error)
        setTreeNodes([])
      } finally {
        setTreeLoading(false)
      }
    }
    loadRootNodes()
  }, [repo, branch, workspace, autoExpandFolder, currentPath])

  // SQL search with debounce
  useEffect(() => {
    if (!search.trim()) {
      setSearchResults([])
      return
    }

    if (searchTimeoutRef.current) {
      clearTimeout(searchTimeoutRef.current)
    }

    searchTimeoutRef.current = setTimeout(async () => {
      if (!repo) return
      setSearchLoading(true)
      try {
        const sql = `
          SELECT id, path, name, properties
          FROM ${workspace}
          WHERE node_type = '${nodeType}'
            AND (
              COALESCE(name, '') ILIKE '%' || $1 || '%'
              OR COALESCE(properties ->> 'title', '') ILIKE '%' || $1 || '%'
              OR COALESCE(path, '') ILIKE '%' || $1 || '%'
            )
          ORDER BY name
          LIMIT 50
        `
        const response = await sqlApi.executeQuery(repo, sql, [search.trim()])
        const results: SearchResult[] = response.rows.map((row) => ({
          id: row.id,
          path: row.path,
          name: row.name,
          title: row.properties?.title,
          description: row.properties?.description,
        }))
        setSearchResults(results)
      } catch (error) {
        console.error('Search failed:', error)
        setSearchResults([])
      } finally {
        setSearchLoading(false)
      }
    }, 300)

    return () => {
      if (searchTimeoutRef.current) {
        clearTimeout(searchTimeoutRef.current)
      }
    }
  }, [search, repo, workspace, nodeType])

  // Load children for a folder
  const loadChildren = useCallback(
    async (nodePath: string) => {
      if (!repo || !branch) return
      if (childrenCache.has(nodePath)) return

      setLoadingPaths((prev) => new Set(prev).add(nodePath))

      try {
        const children = await nodesApi.listChildrenAtHead(repo, branch, workspace, nodePath)
        setChildrenCache((prev) => new Map(prev).set(nodePath, children))
      } catch (error) {
        console.error('Failed to load children:', error)
      } finally {
        setLoadingPaths((prev) => {
          const next = new Set(prev)
          next.delete(nodePath)
          return next
        })
      }
    },
    [repo, branch, workspace, childrenCache]
  )

  // Toggle folder expansion
  const toggleFolder = useCallback(
    (nodePath: string) => {
      const isExpanded = expandedPaths.has(nodePath)

      if (isExpanded) {
        setExpandedPaths((prev) => {
          const next = new Set(prev)
          next.delete(nodePath)
          return next
        })
      } else {
        if (!childrenCache.has(nodePath)) {
          loadChildren(nodePath)
        }
        setExpandedPaths((prev) => new Set(prev).add(nodePath))
      }
    },
    [expandedPaths, childrenCache, loadChildren]
  )

  // Handle node click
  const handleNodeClick = useCallback(
    (node: TreeNode) => {
      if (node.node_type === nodeType) {
        onSelect({
          id: node.id,
          path: node.path,
          name: node.name,
          properties: node.properties as Record<string, unknown>,
        })
      } else if (node.node_type === 'raisin:Folder') {
        toggleFolder(node.path)
      }
    },
    [nodeType, onSelect, toggleFolder]
  )

  // Handle keyboard
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      }
    },
    [onClose]
  )

  // Scroll selected item into view (after tree loads or selection changes)
  useEffect(() => {
    if (listRef.current && selectedPath && !treeLoading) {
      // Small delay to ensure DOM is updated after tree expansion
      const timer = setTimeout(() => {
        const selected = listRef.current?.querySelector(`[data-path="${CSS.escape(selectedPath)}"]`)
        selected?.scrollIntoView({ block: 'nearest' })
      }, 50)
      return () => clearTimeout(timer)
    }
  }, [selectedPath, treeLoading])

  // Get selection background class
  const getSelectionBgClass = (isSelected: boolean) => {
    if (!isSelected) return ''
    // Map color names to their bg classes
    const colorMap: Record<string, string> = {
      'primary-500': 'bg-primary-500/20',
      'purple-500': 'bg-purple-500/20',
      'blue-500': 'bg-blue-500/20',
      'green-500': 'bg-green-500/20',
    }
    return colorMap[selectionColor] || 'bg-primary-500/20'
  }

  // Render a tree node recursively
  const renderTreeNode = (node: TreeNode, depth: number): React.ReactNode => {
    const isFolder = node.node_type === 'raisin:Folder'
    const isTargetType = node.node_type === nodeType
    const isSelectable = isFolder || isTargetType
    const isSelected = node.path === selectedPath
    const isExpanded = expandedPaths.has(node.path)
    const isLoading = loadingPaths.has(node.path)
    const children = childrenCache.get(node.path) || []
    const isCurrent = node.path === currentPath

    // Filter non-matching nodes if configured
    if (filterTreeNodes && !isFolder && !isTargetType) {
      return null
    }

    return (
      <div key={node.id}>
        <button
          data-path={node.path}
          onClick={() => isSelectable && handleNodeClick(node)}
          onMouseEnter={() => setSelectedPath(node.path)}
          className={`
            w-full px-3 py-1.5 flex items-center gap-2 text-left transition-colors text-sm
            ${isSelected ? `${getSelectionBgClass(true)} text-white` : ''}
            ${isSelectable ? 'hover:bg-white/5 text-gray-300' : 'text-gray-600 cursor-default'}
          `}
          style={{ paddingLeft: `${12 + depth * 16}px` }}
          disabled={!isSelectable}
        >
          {/* Expand/collapse icon for folders */}
          {isFolder && (
            <span className="w-4 h-4 flex items-center justify-center flex-shrink-0">
              {isLoading ? (
                <Loader2 className="w-3 h-3 animate-spin text-gray-500" />
              ) : isExpanded ? (
                <ChevronDown className="w-4 h-4 text-gray-500" />
              ) : (
                <ChevronRight className="w-4 h-4 text-gray-500" />
              )}
            </span>
          )}

          {/* Spacer for non-folders to align with folder children */}
          {!isFolder && <span className="w-4" />}

          {/* Node icon */}
          {isFolder ? (
            <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
          ) : isTargetType ? (
            <Icon className={`w-4 h-4 ${iconColor} flex-shrink-0`} />
          ) : (
            <FileText className="w-4 h-4 text-gray-600 flex-shrink-0" />
          )}

          {/* Node name */}
          <span className="truncate flex-1">
            {(node.properties?.title as string) || node.name}
          </span>

          {/* Current indicator */}
          {isCurrent && (
            <span
              className={`text-xs ${iconColor} px-1.5 py-0.5 rounded ${getSelectionBgClass(true)}`}
            >
              current
            </span>
          )}

          {/* Node type badge for non-standard types */}
          {!isFolder && !isTargetType && !filterTreeNodes && (
            <span className="text-xs text-gray-600">{node.node_type.replace('raisin:', '')}</span>
          )}
        </button>

        {/* Render children if expanded */}
        {isFolder && isExpanded && children.length > 0 && (
          <div>{children.map((child) => renderTreeNode(child, depth + 1))}</div>
        )}
      </div>
    )
  }

  // Render search result
  const renderSearchResult = (result: SearchResult) => {
    const isSelected = result.path === selectedPath
    const isCurrent = result.path === currentPath

    return (
      <button
        key={result.id}
        data-path={result.path}
        onClick={() =>
          onSelect({
            id: result.id,
            path: result.path,
            name: result.name,
            properties: result.title ? { title: result.title } : undefined,
          })
        }
        onMouseEnter={() => setSelectedPath(result.path)}
        className={`
          w-full px-4 py-2 flex items-center gap-3 text-left transition-colors
          ${isSelected ? `${getSelectionBgClass(true)} text-white` : 'text-gray-300 hover:bg-white/5'}
        `}
      >
        <Icon className={`w-4 h-4 ${iconColor} flex-shrink-0`} />
        <div className="flex-1 min-w-0">
          <div className="truncate">{result.title || result.name}</div>
          <div className="text-xs text-gray-500 truncate">{result.path}</div>
        </div>
        {isCurrent && (
          <span className={`text-xs ${iconColor} px-1.5 py-0.5 rounded ${getSelectionBgClass(true)}`}>
            current
          </span>
        )}
      </button>
    )
  }

  const isSearchMode = search.trim().length > 0

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh] bg-black/50 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="w-[500px] max-h-[60vh] bg-gray-900 rounded-lg shadow-2xl border border-white/10 flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        {/* Header */}
        <div className="px-4 py-3 border-b border-white/10">
          <h3 className="text-lg font-medium text-white">{title}</h3>
          <p className="text-sm text-gray-400">{subtitle}</p>
        </div>

        {/* Search Input */}
        <div className="flex items-center gap-3 px-4 py-2 border-b border-white/10">
          <Search className="w-4 h-4 text-gray-400" />
          <input
            ref={inputRef}
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={searchPlaceholder}
            className="flex-1 bg-transparent text-white placeholder-gray-500 outline-none text-sm"
          />
          {search && (
            <button
              onClick={() => setSearch('')}
              className="p-1 text-gray-400 hover:text-white transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Content */}
        <div ref={listRef} className="flex-1 overflow-auto py-2">
          {isSearchMode ? (
            // Search Results
            searchLoading ? (
              <div className="p-8 text-center text-gray-400 flex items-center justify-center gap-2">
                <Loader2 className="w-4 h-4 animate-spin" />
                Searching...
              </div>
            ) : searchResults.length === 0 ? (
              <div className="p-8 text-center text-gray-400">
                No results found for "{search}"
              </div>
            ) : (
              searchResults.map((result) => renderSearchResult(result))
            )
          ) : // Tree Browser
          treeLoading ? (
            <div className="p-8 text-center text-gray-400 flex items-center justify-center gap-2">
              <Loader2 className="w-4 h-4 animate-spin" />
              Loading...
            </div>
          ) : treeNodes.length === 0 ? (
            <div className="p-8 text-center text-gray-400">
              <p>{emptyMessage}</p>
              {emptyHint && <p className="text-xs mt-2">{emptyHint}</p>}
            </div>
          ) : (
            treeNodes.map((node) => renderTreeNode(node, 0))
          )}
        </div>

        {/* Footer */}
        <div className="px-4 py-2 border-t border-white/10 flex items-center justify-between">
          <div className="text-xs text-gray-500">
            {isSearchMode ? 'Type to search' : 'Click folders to expand'}
          </div>
          <button
            onClick={onClose}
            className="px-3 py-1 text-sm text-gray-400 hover:text-white transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
