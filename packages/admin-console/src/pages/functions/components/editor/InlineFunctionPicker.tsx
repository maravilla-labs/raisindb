/**
 * InlineFunctionPicker Component
 *
 * Inline dropdown for selecting functions with search and tree browsing.
 * Used in RaisinAgentNodeTypeEditor for tool selection.
 */

import { useState, useEffect, useCallback, useRef } from 'react'
import { Search, X, Folder, Play, ChevronRight, ChevronDown, Loader2, AlertTriangle } from 'lucide-react'
import { createPortal } from 'react-dom'
import { nodesApi, type Node } from '../../../../api/nodes'
import { sqlApi } from '../../../../api/sql'
import { useRepositoryContext } from '../../../../hooks/useRepositoryContext'

interface InlineFunctionPickerProps {
  value: string[]
  onChange: (paths: string[]) => void
  disabled?: boolean
  disabledMessage?: string
  placeholder?: string
  label?: string
  helperText?: string
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
}

const FUNCTIONS_WORKSPACE = 'functions'

export function InlineFunctionPicker({
  value,
  onChange,
  disabled = false,
  disabledMessage,
  placeholder = 'Search functions...',
  label = 'Tools',
  helperText,
}: InlineFunctionPickerProps) {
  const { repo, branch } = useRepositoryContext()
  const [isOpen, setIsOpen] = useState(false)
  const [search, setSearch] = useState('')
  const [treeNodes, setTreeNodes] = useState<TreeNode[]>([])
  const [searchResults, setSearchResults] = useState<SearchResult[]>([])
  const [treeLoading, setTreeLoading] = useState(false)
  const [searchLoading, setSearchLoading] = useState(false)
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(new Set())
  const [childrenCache, setChildrenCache] = useState<Map<string, TreeNode[]>>(new Map())
  const [highlightedIndex, setHighlightedIndex] = useState(-1)
  const [dropdownPosition, setDropdownPosition] = useState({ top: 0, left: 0, width: 0 })

  const containerRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Load root nodes when dropdown opens
  useEffect(() => {
    if (!isOpen || !repo || !branch) return

    const loadRootNodes = async () => {
      setTreeLoading(true)
      try {
        const data = await nodesApi.listRootAtHead(repo, branch, FUNCTIONS_WORKSPACE)
        setTreeNodes(data)
      } catch (error) {
        console.error('Failed to load nodes:', error)
        setTreeNodes([])
      } finally {
        setTreeLoading(false)
      }
    }

    loadRootNodes()
  }, [isOpen, repo, branch])

  // SQL search with debounce
  useEffect(() => {
    if (!isOpen || !search.trim()) {
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
          FROM ${FUNCTIONS_WORKSPACE}
          WHERE node_type = 'raisin:Function'
            AND (
              COALESCE(name, '') ILIKE '%' || $1 || '%'
              OR COALESCE(properties ->> 'title', '') ILIKE '%' || $1 || '%'
              OR COALESCE(path, '') ILIKE '%' || $1 || '%'
            )
          ORDER BY name
          LIMIT 50
        `
        const response = await sqlApi.executeQuery(repo, sql, [search.trim()])
        const results: SearchResult[] = response.rows.map(row => ({
          id: row.id,
          path: row.path,
          name: row.name,
          title: row.properties?.title,
        }))
        setSearchResults(results)
        setHighlightedIndex(results.length > 0 ? 0 : -1)
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
  }, [search, repo, isOpen])

  // Load children for a folder
  const loadChildren = useCallback(async (nodePath: string) => {
    if (!repo || !branch) return
    if (childrenCache.has(nodePath)) return

    setLoadingPaths(prev => new Set(prev).add(nodePath))

    try {
      const children = await nodesApi.listChildrenAtHead(repo, branch, FUNCTIONS_WORKSPACE, nodePath)
      setChildrenCache(prev => new Map(prev).set(nodePath, children))
    } catch (error) {
      console.error('Failed to load children:', error)
    } finally {
      setLoadingPaths(prev => {
        const next = new Set(prev)
        next.delete(nodePath)
        return next
      })
    }
  }, [repo, branch, childrenCache])

  // Toggle folder expansion
  const toggleFolder = useCallback((nodePath: string) => {
    const isExpanded = expandedPaths.has(nodePath)

    if (isExpanded) {
      setExpandedPaths(prev => {
        const next = new Set(prev)
        next.delete(nodePath)
        return next
      })
    } else {
      if (!childrenCache.has(nodePath)) {
        loadChildren(nodePath)
      }
      setExpandedPaths(prev => new Set(prev).add(nodePath))
    }
  }, [expandedPaths, childrenCache, loadChildren])

  // Handle function selection
  const handleSelect = useCallback((path: string) => {
    if (value.includes(path)) {
      // Already selected, remove it
      onChange(value.filter(p => p !== path))
    } else {
      // Add to selection
      onChange([...value, path])
    }
  }, [value, onChange])

  // Handle removing a selected function
  const handleRemove = useCallback((path: string) => {
    onChange(value.filter(p => p !== path))
  }, [value, onChange])

  // Update dropdown position
  const updatePosition = useCallback(() => {
    if (containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect()
      setDropdownPosition({
        top: rect.bottom + window.scrollY + 4,
        left: rect.left + window.scrollX,
        width: rect.width,
      })
    }
  }, [])

  // Handle opening/closing
  const handleOpen = useCallback(() => {
    if (disabled) return
    setIsOpen(true)
    updatePosition()
    setTimeout(() => inputRef.current?.focus(), 0)
  }, [disabled, updatePosition])

  const handleClose = useCallback(() => {
    setIsOpen(false)
    setSearch('')
    setHighlightedIndex(-1)
  }, [])

  // Close on click outside
  useEffect(() => {
    if (!isOpen) return

    const handleClickOutside = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as globalThis.Node) &&
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as globalThis.Node)
      ) {
        handleClose()
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [isOpen, handleClose])

  // Handle keyboard navigation
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      handleClose()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      const items = search.trim() ? searchResults : []
      if (items.length > 0) {
        setHighlightedIndex(prev => Math.min(prev + 1, items.length - 1))
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setHighlightedIndex(prev => Math.max(prev - 1, 0))
    } else if (e.key === 'Enter' && highlightedIndex >= 0) {
      e.preventDefault()
      const items = search.trim() ? searchResults : []
      if (items[highlightedIndex]) {
        handleSelect(items[highlightedIndex].path)
      }
    }
  }, [search, searchResults, highlightedIndex, handleSelect, handleClose])

  // Render tree node recursively
  const renderTreeNode = (node: TreeNode, depth: number) => {
    const isFolder = node.node_type === 'raisin:Folder'
    const isFunction = node.node_type === 'raisin:Function'
    const isSelectable = isFunction
    const isSelected = value.includes(node.path)
    const isExpanded = expandedPaths.has(node.path)
    const isLoading = loadingPaths.has(node.path)
    const children = childrenCache.get(node.path) || []

    return (
      <div key={node.id}>
        <button
          type="button"
          onClick={() => {
            if (isFolder) {
              toggleFolder(node.path)
            } else if (isFunction) {
              handleSelect(node.path)
            }
          }}
          className={`
            w-full px-3 py-1.5 flex items-center gap-2 text-left transition-colors text-sm
            ${isSelected ? 'bg-purple-500/20 text-purple-300' : ''}
            ${isSelectable && !isSelected ? 'hover:bg-white/5 text-gray-300' : ''}
            ${isFolder ? 'hover:bg-white/5 text-gray-300' : ''}
            ${!isSelectable && !isFolder ? 'text-gray-600 cursor-default' : ''}
          `}
          style={{ paddingLeft: `${12 + depth * 16}px` }}
        >
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

          {!isFolder && <span className="w-4" />}

          {isFolder ? (
            <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
          ) : isFunction ? (
            <Play className="w-4 h-4 text-blue-400 flex-shrink-0" />
          ) : null}

          <span className="truncate flex-1">
            {(node.properties?.title as string) || node.name}
          </span>

          {isSelected && (
            <span className="text-xs text-purple-400">✓</span>
          )}
        </button>

        {isFolder && isExpanded && children.length > 0 && (
          <div>
            {children.map(child => renderTreeNode(child, depth + 1))}
          </div>
        )}
      </div>
    )
  }

  // Render search result
  const renderSearchResult = (result: SearchResult, index: number) => {
    const isSelected = value.includes(result.path)
    const isHighlighted = index === highlightedIndex

    return (
      <button
        key={result.id}
        type="button"
        onClick={() => handleSelect(result.path)}
        onMouseEnter={() => setHighlightedIndex(index)}
        className={`
          w-full px-4 py-2 flex items-center gap-3 text-left transition-colors
          ${isHighlighted ? 'bg-white/10' : ''}
          ${isSelected ? 'bg-purple-500/20 text-purple-300' : 'text-gray-300 hover:bg-white/5'}
        `}
      >
        <Play className="w-4 h-4 text-blue-400 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="truncate">{result.title || result.name}</div>
          <div className="text-xs text-gray-500 truncate">{result.path}</div>
        </div>
        {isSelected && (
          <span className="text-xs text-purple-400">✓</span>
        )}
      </button>
    )
  }

  const isSearchMode = search.trim().length > 0

  return (
    <div ref={containerRef} className="relative">
      {/* Label */}
      {label && (
        <label className="block text-sm font-medium text-zinc-300 mb-2">
          {label}
        </label>
      )}

      {/* Selected items as chips */}
      {value.length > 0 && (
        <div className="flex flex-wrap gap-2 mb-2">
          {value.map(path => {
            const name = path.split('/').pop() || path
            return (
              <span
                key={path}
                className="inline-flex items-center gap-1 px-2 py-1 bg-purple-500/20 text-purple-300 rounded text-sm"
              >
                <Play className="w-3 h-3" />
                <span className="truncate max-w-[150px]">{name}</span>
                {!disabled && (
                  <button
                    type="button"
                    onClick={() => handleRemove(path)}
                    className="p-0.5 hover:bg-purple-500/30 rounded"
                  >
                    <X className="w-3 h-3" />
                  </button>
                )}
              </span>
            )
          })}
        </div>
      )}

      {/* Input trigger */}
      <div
        onClick={handleOpen}
        className={`
          w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white
          cursor-text flex items-center gap-2
          ${disabled ? 'opacity-50 cursor-not-allowed' : 'focus-within:border-purple-500 focus-within:ring-2 focus-within:ring-purple-500/20'}
        `}
      >
        <Search className="w-4 h-4 text-gray-400" />
        {isOpen ? (
          <input
            ref={inputRef}
            type="text"
            value={search}
            onChange={e => setSearch(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            className="flex-1 bg-transparent outline-none placeholder-zinc-500 text-sm"
            disabled={disabled}
          />
        ) : (
          <span className="flex-1 text-zinc-500 text-sm">{placeholder}</span>
        )}
      </div>

      {/* Disabled warning */}
      {disabled && disabledMessage && (
        <div className="mt-2 flex items-center gap-2 text-amber-400 text-sm">
          <AlertTriangle className="w-4 h-4" />
          {disabledMessage}
        </div>
      )}

      {/* Helper text */}
      {helperText && !disabled && (
        <p className="text-xs text-zinc-500 mt-1">{helperText}</p>
      )}

      {/* Dropdown portal */}
      {isOpen && createPortal(
        <div
          ref={dropdownRef}
          className="fixed z-50 bg-gray-900 border border-white/10 rounded-lg shadow-2xl overflow-hidden"
          style={{
            top: dropdownPosition.top,
            left: dropdownPosition.left,
            width: dropdownPosition.width,
            maxHeight: '300px',
          }}
        >
          <div className="overflow-auto max-h-[280px]">
            {isSearchMode ? (
              searchLoading ? (
                <div className="p-4 text-center text-gray-400 flex items-center justify-center gap-2">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Searching...
                </div>
              ) : searchResults.length === 0 ? (
                <div className="p-4 text-center text-gray-400">
                  No functions found for "{search}"
                </div>
              ) : (
                searchResults.map((result, index) => renderSearchResult(result, index))
              )
            ) : treeLoading ? (
              <div className="p-4 text-center text-gray-400 flex items-center justify-center gap-2">
                <Loader2 className="w-4 h-4 animate-spin" />
                Loading...
              </div>
            ) : treeNodes.length === 0 ? (
              <div className="p-4 text-center text-gray-400">
                No functions in workspace
              </div>
            ) : (
              <div className="py-2">
                {treeNodes.map(node => renderTreeNode(node, 0))}
              </div>
            )}
          </div>

          {/* Footer hint */}
          <div className="px-3 py-1.5 border-t border-white/10 text-xs text-gray-500">
            {isSearchMode ? 'Type to search • Click to toggle' : 'Click folders to expand • Click functions to toggle'}
          </div>
        </div>,
        document.body
      )}
    </div>
  )
}
