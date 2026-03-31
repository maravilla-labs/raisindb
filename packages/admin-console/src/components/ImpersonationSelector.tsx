import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import {
  UserCircle,
  Eye,
  EyeOff,
  Check,
  ChevronDown,
  ChevronRight,
  Search,
  AlertTriangle,
  Folder,
  Loader2
} from 'lucide-react'
import { nodesApi, type Node } from '../api/nodes'
import { sqlApi } from '../api/sql'
import { getImpersonatedUserId, getImpersonatedUserName, setImpersonatedUserId } from '../api/client'
import { useAuth } from '../contexts/AuthContext'

interface ImpersonationSelectorProps {
  /** Repository name */
  repo: string
  /** Optional className for styling */
  className?: string
  /** Callback when impersonation changes */
  onChange?: (userId: string | null) => void
}

interface UserSearchResult {
  id: string
  path: string
  user_id: string
  email: string
  display_name: string
}

const ACCESS_CONTROL_WORKSPACE = 'raisin:access_control'

export default function ImpersonationSelector({
  repo,
  className = '',
  onChange
}: ImpersonationSelectorProps) {
  const { user: adminUser } = useAuth()
  const [isOpen, setIsOpen] = useState(false)
  const [searchTerm, setSearchTerm] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [impersonatedUserId, setImpersonatedUserIdState] = useState<string | null>(
    getImpersonatedUserId()
  )
  const [buttonRect, setButtonRect] = useState<DOMRect | null>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const buttonRef = useRef<HTMLButtonElement>(null)

  // Tree browsing state
  const [treeNodes, setTreeNodes] = useState<Node[]>([])
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())
  const [loadingChildren, setLoadingChildren] = useState<Set<string>>(new Set())
  const [childrenCache, setChildrenCache] = useState<Map<string, Node[]>>(new Map())
  const [treeLoading, setTreeLoading] = useState(false)

  // SQL search state
  const [searchResults, setSearchResults] = useState<UserSearchResult[]>([])
  const [searchLoading, setSearchLoading] = useState(false)
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Check if current admin can impersonate
  const canImpersonate = adminUser?.access_flags?.can_impersonate ?? false

  // Determine if we're in search mode
  const isSearchMode = searchTerm.trim().length > 0

  // Load root nodes when dropdown opens
  useEffect(() => {
    if (isOpen && canImpersonate) {
      loadRootNodes()
    }
  }, [isOpen, canImpersonate, repo])

  // Handle search with debounce
  useEffect(() => {
    if (searchTimeoutRef.current) {
      clearTimeout(searchTimeoutRef.current)
      searchTimeoutRef.current = null
    }

    if (!searchTerm.trim()) {
      setSearchResults([])
      return
    }

    searchTimeoutRef.current = setTimeout(() => {
      performSqlSearch(searchTerm.trim())
    }, 350)

    return () => {
      if (searchTimeoutRef.current) {
        clearTimeout(searchTimeoutRef.current)
      }
    }
  }, [searchTerm, repo])

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (searchTimeoutRef.current) {
        clearTimeout(searchTimeoutRef.current)
      }
    }
  }, [])

  // Close dropdown when clicking outside
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as globalThis.Node)) {
        setIsOpen(false)
      }
    }
    if (isOpen) {
      if (buttonRef.current) {
        setButtonRect(buttonRef.current.getBoundingClientRect())
      }
      document.addEventListener('mousedown', handleClickOutside)
      return () => document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [isOpen])

  async function loadRootNodes() {
    if (!repo) return
    setTreeLoading(true)
    setError(null)
    try {
      const nodes = await nodesApi.listRootAtHead(repo, 'main', ACCESS_CONTROL_WORKSPACE)
      setTreeNodes(nodes)
    } catch (err) {
      console.error('Failed to load root nodes for impersonation:', err)
      setError('Failed to load users')
    } finally {
      setTreeLoading(false)
    }
  }

  async function loadChildren(nodePath: string) {
    if (childrenCache.has(nodePath)) return

    setLoadingChildren(prev => new Set(prev).add(nodePath))
    try {
      const children = await nodesApi.listChildrenAtHead(repo, 'main', ACCESS_CONTROL_WORKSPACE, nodePath)
      setChildrenCache(prev => new Map(prev).set(nodePath, children))
    } catch (err) {
      console.error('Failed to load children:', err)
    } finally {
      setLoadingChildren(prev => {
        const next = new Set(prev)
        next.delete(nodePath)
        return next
      })
    }
  }

  function toggleFolder(node: Node) {
    const isExpanded = expandedNodes.has(node.path)

    if (isExpanded) {
      setExpandedNodes(prev => {
        const next = new Set(prev)
        next.delete(node.path)
        return next
      })
    } else {
      if (!childrenCache.has(node.path)) {
        loadChildren(node.path)
      }
      setExpandedNodes(prev => new Set(prev).add(node.path))
    }
  }

  async function performSqlSearch(term: string) {
    if (!repo) return

    setSearchLoading(true)
    setError(null)
    try {
      const sql = `
        SELECT id, path, name,
               properties->>'user_id' as user_id,
               properties->>'email' as email,
               properties->>'display_name' as display_name
        FROM "raisin:access_control"
        WHERE node_type = 'raisin:User'
          AND (
            COALESCE(properties->>'display_name', '') ILIKE '%' || $1 || '%'
            OR COALESCE(properties->>'email', '') ILIKE '%' || $1 || '%'
            OR COALESCE(properties->>'user_id', '') ILIKE '%' || $1 || '%'
            OR COALESCE(name, '') ILIKE '%' || $1 || '%'
          )
        ORDER BY properties->>'display_name'
        LIMIT 50
      `
      const response = await sqlApi.executeQuery(repo, sql, [term])
      const results: UserSearchResult[] = response.rows.map(row => ({
        id: row.id,
        path: row.path,
        user_id: row.user_id || row.name,
        email: row.email || '',
        display_name: row.display_name || row.name,
      }))
      setSearchResults(results)
    } catch (err) {
      console.error('User search failed:', err)
      setSearchResults([])
    } finally {
      setSearchLoading(false)
    }
  }

  function handleSearchKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && searchTerm.trim()) {
      e.preventDefault()
      if (searchTimeoutRef.current) {
        clearTimeout(searchTimeoutRef.current)
        searchTimeoutRef.current = null
      }
      performSqlSearch(searchTerm.trim())
    }
  }

  function handleSelectUser(userId: string | null, displayName?: string) {
    setImpersonatedUserId(userId, displayName || undefined)
    setImpersonatedUserIdState(userId)
    setIsOpen(false)
    setSearchTerm('')
    onChange?.(userId)
  }

  // Get display name for currently impersonated user from search results or tree
  function getImpersonatedDisplayName(): string {
    if (!impersonatedUserId) return ''

    // Check search results first (match by node id)
    const searchResult = searchResults.find(r => r.id === impersonatedUserId)
    if (searchResult) return searchResult.display_name

    // Check tree nodes recursively (match by node id)
    function findInNodes(nodes: Node[]): string | null {
      for (const node of nodes) {
        if (node.node_type === 'raisin:User') {
          if (node.id === impersonatedUserId) {
            return (node.properties?.display_name as string) || node.name
          }
        }
        const children = childrenCache.get(node.path)
        if (children) {
          const found = findInNodes(children)
          if (found) return found
        }
      }
      return null
    }

    const found = findInNodes(treeNodes)
    if (found) return found

    // Fallback to stored display name from localStorage
    const storedName = getImpersonatedUserName()
    return storedName || impersonatedUserId
  }

  const impersonatedName = getImpersonatedDisplayName()

  function renderSearchResult(result: UserSearchResult) {
    const isSelected = result.id === impersonatedUserId

    return (
      <button
        key={result.id}
        onClick={() => handleSelectUser(result.id, result.display_name)}
        className={`
          w-full flex items-center gap-3 px-4 py-2
          hover:bg-white/5 transition-colors text-left
          ${isSelected ? 'bg-primary-500/10' : ''}
        `}
      >
        <UserCircle className="w-5 h-5 text-gray-400 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-white text-sm font-medium truncate">
              {result.display_name}
            </span>
            {isSelected && (
              <Check className="w-4 h-4 text-green-400 flex-shrink-0" />
            )}
          </div>
          {result.email && result.email !== result.display_name && (
            <p className="text-xs text-gray-400 truncate">{result.email}</p>
          )}
          <p className="text-xs text-gray-500 truncate">{result.path}</p>
        </div>
        <span className="text-xs text-gray-500 truncate max-w-20">
          {result.user_id}
        </span>
      </button>
    )
  }

  function renderTreeNode(node: Node, depth: number = 0): React.ReactNode {
    const isFolder = node.node_type === 'raisin:AclFolder'
    const isUser = node.node_type === 'raisin:User'

    // Skip nodes that are neither folders nor users
    if (!isFolder && !isUser) return null

    const isExpanded = expandedNodes.has(node.path)
    const isLoadingThis = loadingChildren.has(node.path)
    const children = childrenCache.get(node.path) || []
    const hasChildren = node.has_children || children.length > 0

    // Use node.id (UUID) for impersonation, but keep userId property for display
    const nodeId = node.id
    const userIdDisplay = isUser ? ((node.properties?.user_id as string) || node.name) : null
    const isSelected = isUser && nodeId === impersonatedUserId
    const displayName = isUser
      ? ((node.properties?.display_name as string) || node.name)
      : node.name
    const email = isUser ? (node.properties?.email as string) : null

    return (
      <div key={node.id}>
        <button
          onClick={() => isFolder ? toggleFolder(node) : handleSelectUser(nodeId, displayName)}
          className={`
            w-full flex items-center gap-2 px-4 py-2
            hover:bg-white/5 transition-colors text-left
            ${isSelected ? 'bg-primary-500/10' : ''}
          `}
          style={{ paddingLeft: `${16 + depth * 16}px` }}
        >
          {/* Expand/collapse icon for folders */}
          {isFolder && hasChildren ? (
            isLoadingThis ? (
              <Loader2 className="w-4 h-4 animate-spin text-gray-400 flex-shrink-0" />
            ) : isExpanded ? (
              <ChevronDown className="w-4 h-4 text-gray-400 flex-shrink-0" />
            ) : (
              <ChevronRight className="w-4 h-4 text-gray-400 flex-shrink-0" />
            )
          ) : isFolder ? (
            <div className="w-4 flex-shrink-0" />
          ) : null}

          {/* Icon */}
          {isFolder ? (
            <Folder className="w-4 h-4 text-amber-400 flex-shrink-0" />
          ) : (
            <UserCircle className="w-4 h-4 text-gray-400 flex-shrink-0" />
          )}

          {/* Node content */}
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-white text-sm truncate">{displayName}</span>
              {isSelected && (
                <Check className="w-4 h-4 text-green-400 flex-shrink-0" />
              )}
            </div>
            {isUser && email && email !== displayName && (
              <p className="text-xs text-gray-400 truncate">{email}</p>
            )}
          </div>

          {isUser && (
            <span className="text-xs text-gray-500 truncate max-w-20">
              {userIdDisplay}
            </span>
          )}
        </button>

        {/* Render children if expanded */}
        {isFolder && isExpanded && children.length > 0 && (
          <div>
            {children.map(child => renderTreeNode(child, depth + 1))}
          </div>
        )}
      </div>
    )
  }

  // Don't render if user can't impersonate
  if (!canImpersonate) {
    return null
  }

  return (
    <div className={`relative ${className}`}>
      {/* Trigger Button */}
      <button
        ref={buttonRef}
        onClick={() => setIsOpen(!isOpen)}
        className={`
          flex items-center gap-2 px-3 py-1.5
          ${impersonatedUserId
            ? 'bg-amber-500/20 hover:bg-amber-500/30 border-amber-500/50 hover:border-amber-500/70'
            : 'bg-black/30 hover:bg-black/40 border-white/20 hover:border-white/30'
          }
          border rounded-lg text-white transition-colors text-sm
        `}
        title={impersonatedUserId ? `Viewing as: ${impersonatedName}` : 'View as user'}
      >
        {impersonatedUserId ? (
          <>
            <Eye className="w-4 h-4 text-amber-400" />
            <span className="text-amber-300 max-w-32 truncate">{impersonatedName}</span>
          </>
        ) : (
          <>
            <EyeOff className="w-4 h-4 text-gray-400" />
            <span className="text-gray-300">View as...</span>
          </>
        )}
        <ChevronDown className={`w-4 h-4 text-gray-400 transition-transform ${isOpen ? 'rotate-180' : ''}`} />
      </button>

      {/* Dropdown Menu */}
      {isOpen && buttonRect && createPortal(
        <div
          ref={dropdownRef}
          className="fixed w-80 bg-zinc-900 border border-white/20 rounded-lg shadow-2xl overflow-hidden"
          style={{
            top: `${buttonRect.bottom + 8}px`,
            left: `${Math.max(8, buttonRect.left - 100)}px`,
            zIndex: 9999
          }}
        >
          {/* Header */}
          <div className="px-4 py-3 bg-black/30 border-b border-white/10">
            <div className="flex items-center gap-2 text-white font-medium">
              <UserCircle className="w-5 h-5 text-primary-400" />
              User Impersonation
            </div>
            <p className="text-xs text-gray-400 mt-1">
              View content as a specific user to test permissions
            </p>
          </div>

          {/* Search */}
          <div className="p-3 border-b border-white/10">
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
              <input
                type="text"
                placeholder="Search users..."
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
                onKeyDown={handleSearchKeyDown}
                className="w-full pl-10 pr-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white text-sm placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
                autoFocus
              />
            </div>
          </div>

          {/* Clear impersonation option */}
          {impersonatedUserId && (
            <button
              onClick={() => handleSelectUser(null)}
              className="w-full flex items-center gap-3 px-4 py-3 hover:bg-white/5 transition-colors text-left border-b border-white/10 bg-amber-500/10"
            >
              <EyeOff className="w-4 h-4 text-amber-400 flex-shrink-0" />
              <div className="flex-1">
                <span className="text-amber-300 text-sm font-medium">
                  Exit impersonation
                </span>
                <p className="text-xs text-gray-400">Return to admin view</p>
              </div>
            </button>
          )}

          {/* Content Area - Tree Browse or Search Results */}
          <div className="max-h-80 overflow-y-auto">
            {isSearchMode ? (
              // SQL Search Results Mode
              searchLoading ? (
                <div className="p-8 text-center text-gray-400">
                  <Loader2 className="w-6 h-6 animate-spin mx-auto mb-2" />
                  Searching users...
                </div>
              ) : error ? (
                <div className="p-8 text-center text-red-400">
                  <AlertTriangle className="w-6 h-6 mx-auto mb-2" />
                  {error}
                </div>
              ) : searchResults.length > 0 ? (
                <div className="py-2">
                  {searchResults.map(result => renderSearchResult(result))}
                </div>
              ) : (
                <div className="p-8 text-center text-gray-400">
                  No users match "{searchTerm}"
                </div>
              )
            ) : (
              // Tree Browse Mode
              treeLoading ? (
                <div className="p-8 text-center text-gray-400">
                  <Loader2 className="w-6 h-6 animate-spin mx-auto mb-2" />
                  Loading users...
                </div>
              ) : error ? (
                <div className="p-8 text-center text-red-400">
                  <AlertTriangle className="w-6 h-6 mx-auto mb-2" />
                  {error}
                </div>
              ) : treeNodes.length > 0 ? (
                <div className="py-2">
                  {treeNodes.map(node => renderTreeNode(node, 0))}
                </div>
              ) : (
                <div className="p-8 text-center text-gray-400">
                  No users found
                </div>
              )
            )}
          </div>

          {/* Warning footer */}
          {impersonatedUserId && (
            <div className="px-4 py-2 bg-amber-500/10 border-t border-amber-500/20 text-xs text-amber-300 flex items-center gap-2">
              <AlertTriangle className="w-3 h-3" />
              You are viewing content as another user. Actions are still audited under your account.
            </div>
          )}
        </div>,
        document.body
      )}
    </div>
  )
}
