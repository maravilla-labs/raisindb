import { useState, useEffect } from 'react'
import { X, Folder, ChevronRight, ChevronDown, Home } from 'lucide-react'
import { sqlApi } from '../api/sql'

interface FolderNode {
  path: string
  name: string
  depth: number
  children: FolderNode[]
  isExpanded?: boolean
}

interface MoveToFolderDialogProps {
  repo: string
  branch: string
  itemName: string
  itemType: 'folder' | 'user' | 'role' | 'group' | 'circle'
  currentPath: string  // Current path of the item being moved
  basePath: string     // Base path for this item type (e.g., '/users', '/roles', '/groups')
  onClose: () => void
  onMove: (destinationPath: string) => Promise<void>
}

export default function MoveToFolderDialog({
  repo,
  branch,
  itemName,
  itemType,
  currentPath,
  basePath,
  onClose,
  onMove,
}: MoveToFolderDialogProps) {
  const [folders, setFolders] = useState<FolderNode[]>([])
  const [loading, setLoading] = useState(true)
  const [moving, setMoving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())

  // Get the parent path of current item (to exclude it from destinations)
  const currentParentPath = currentPath.substring(0, currentPath.lastIndexOf('/')) || basePath

  useEffect(() => {
    loadFolders()
  }, [repo, branch, basePath])

  async function loadFolders() {
    setLoading(true)
    setError(null)

    try {
      // Query all AclFolder nodes under the base path
      const sql = `
        SELECT path, name
        FROM "raisin:access_control"
        WHERE node_type = 'raisin:AclFolder'
          AND path LIKE $1
        ORDER BY path
      `
      const result = await sqlApi.executeQuery(repo, sql, [`${basePath}%`])

      // Build hierarchical structure
      const folderMap = new Map<string, FolderNode>()
      const rootFolders: FolderNode[] = []

      // First pass: create all folder nodes
      for (const row of result.rows) {
        const path = row.path as string
        const name = row.name as string
        const depth = path.split('/').length - basePath.split('/').length

        folderMap.set(path, {
          path,
          name,
          depth,
          children: [],
        })
      }

      // Second pass: build hierarchy
      for (const [path, folder] of folderMap) {
        const parentPath = path.substring(0, path.lastIndexOf('/'))
        const parent = folderMap.get(parentPath)

        if (parent) {
          parent.children.push(folder)
        } else {
          rootFolders.push(folder)
        }
      }

      setFolders(rootFolders)
    } catch (err) {
      console.error('Failed to load folders:', err)
      setError('Failed to load folders')
    } finally {
      setLoading(false)
    }
  }

  function toggleExpand(path: string) {
    setExpandedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
      }
      return next
    })
  }

  function isValidDestination(path: string): boolean {
    // Can't move to current location
    if (path === currentParentPath) return false
    // Can't move to self (for folders)
    if (path === currentPath) return false
    // Can't move to a descendant of self (for folders)
    if (itemType === 'folder' && path.startsWith(currentPath + '/')) return false
    return true
  }

  async function handleMove() {
    if (!selectedPath) return

    setMoving(true)
    setError(null)

    try {
      await onMove(selectedPath)
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to move item')
    } finally {
      setMoving(false)
    }
  }

  function renderFolder(folder: FolderNode, level: number = 0): React.ReactNode {
    const isExpanded = expandedPaths.has(folder.path)
    const hasChildren = folder.children.length > 0
    const isSelected = selectedPath === folder.path
    const isDisabled = !isValidDestination(folder.path)
    const isCurrent = folder.path === currentParentPath

    return (
      <div key={folder.path}>
        <button
          type="button"
          onClick={() => {
            if (hasChildren) toggleExpand(folder.path)
            if (!isDisabled) setSelectedPath(folder.path)
          }}
          disabled={isDisabled}
          className={`
            w-full flex items-center gap-2 px-3 py-2 rounded-lg transition-colors text-left
            ${isSelected ? 'bg-primary-500/20 border border-primary-500/50' : 'hover:bg-white/5'}
            ${isDisabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
            ${isCurrent ? 'ring-1 ring-yellow-500/50' : ''}
          `}
          style={{ paddingLeft: `${level * 20 + 12}px` }}
        >
          {hasChildren ? (
            <span className="w-4 h-4 flex items-center justify-center">
              {isExpanded ? (
                <ChevronDown className="w-4 h-4 text-zinc-400" />
              ) : (
                <ChevronRight className="w-4 h-4 text-zinc-400" />
              )}
            </span>
          ) : (
            <span className="w-4" />
          )}
          <Folder className="w-4 h-4 text-primary-400" />
          <span className={`flex-1 truncate ${isSelected ? 'text-white' : 'text-zinc-300'}`}>
            {folder.name}
          </span>
          {isCurrent && (
            <span className="text-xs text-yellow-400">(current)</span>
          )}
        </button>
        {hasChildren && isExpanded && (
          <div>
            {folder.children.map((child) => renderFolder(child, level + 1))}
          </div>
        )}
      </div>
    )
  }

  const rootLabel = basePath === '/users' ? 'Users' : basePath === '/roles' ? 'Roles' : basePath === '/groups' ? 'Groups' : 'Root'
  const isRootSelected = selectedPath === basePath
  const isRootDisabled = !isValidDestination(basePath)
  const isRootCurrent = basePath === currentParentPath

  return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-zinc-900 border border-white/10 rounded-xl shadow-2xl max-w-lg w-full max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div>
            <h2 className="text-xl font-bold text-white">Move {itemType}</h2>
            <p className="text-sm text-zinc-400 mt-1">
              Select a destination folder for "{itemName}"
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-5 h-5 text-white/60" />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-4">
          {error && (
            <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 text-sm">
              {error}
            </div>
          )}

          {loading ? (
            <div className="text-center text-zinc-400 py-8">Loading folders...</div>
          ) : (
            <div className="space-y-1">
              {/* Root option */}
              <button
                type="button"
                onClick={() => !isRootDisabled && setSelectedPath(basePath)}
                disabled={isRootDisabled}
                className={`
                  w-full flex items-center gap-2 px-3 py-2 rounded-lg transition-colors text-left
                  ${isRootSelected ? 'bg-primary-500/20 border border-primary-500/50' : 'hover:bg-white/5'}
                  ${isRootDisabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
                  ${isRootCurrent ? 'ring-1 ring-yellow-500/50' : ''}
                `}
              >
                <span className="w-4" />
                <Home className="w-4 h-4 text-primary-400" />
                <span className={`flex-1 ${isRootSelected ? 'text-white' : 'text-zinc-300'}`}>
                  {rootLabel} (root)
                </span>
                {isRootCurrent && (
                  <span className="text-xs text-yellow-400">(current)</span>
                )}
              </button>

              {/* Folder tree */}
              {folders.map((folder) => renderFolder(folder))}

              {folders.length === 0 && (
                <p className="text-center text-zinc-500 py-4 text-sm">
                  No folders available. Create a folder first.
                </p>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex gap-3 p-6 border-t border-white/10">
          <button
            type="button"
            onClick={onClose}
            className="flex-1 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleMove}
            disabled={moving || !selectedPath}
            className="flex-1 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {moving ? 'Moving...' : 'Move Here'}
          </button>
        </div>
      </div>
    </div>
  )
}
