import { useEffect, useState, useCallback } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { User as UserIcon, Plus, FolderPlus } from 'lucide-react'
import { monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import GlassCard from '../components/GlassCard'
import Breadcrumb from '../components/Breadcrumb'
import FolderCard from '../components/FolderCard'
import FolderDialog from '../components/FolderDialog'
import MoveToFolderDialog from '../components/MoveToFolderDialog'
import ConfirmDialog from '../components/ConfirmDialog'
import { DraggableCardWrapper } from '../components/DraggableCardWrapper'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest } from '../api/nodes'
import type { DragData, DropPosition } from '../hooks/useDraggableCard'
import UserEditor from './UserEditor'

const WORKSPACE = 'raisin:access_control'

interface User {
  id?: string
  name: string        // node name
  path: string        // full path within workspace
  user_id: string
  email: string
  display_name: string
  groups?: string[]
  roles?: string[]
  metadata?: Record<string, unknown>
  created_at?: string
  updated_at?: string
}

function nodeToUser(node: Node): User {
  return {
    id: node.id,
    name: node.name,
    path: node.path,
    user_id: node.properties?.user_id as string,
    email: node.properties?.email as string,
    display_name: node.properties?.display_name as string,
    groups: node.properties?.groups as string[] | undefined,
    roles: node.properties?.roles as string[] | undefined,
    metadata: node.properties?.metadata as Record<string, unknown> | undefined,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

export default function Users() {
  const navigate = useNavigate()
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const basePath = '/users'
  const wildcardPath = pathParam ? `/${pathParam}` : ''
  const currentPath = wildcardPath
    ? wildcardPath.startsWith(basePath)
      ? wildcardPath
      : `${basePath}${wildcardPath}`
    : basePath
  const relativeCurrentPath = currentPath === basePath ? '' : currentPath.slice(basePath.length)

  const [folders, setFolders] = useState<Node[]>([])
  const [users, setUsers] = useState<User[]>([])
  const [loading, setLoading] = useState(true)
  const [showFolderDialog, setShowFolderDialog] = useState(false)
  const [editingFolder, setEditingFolder] = useState<Node | undefined>(undefined)
  const [deleteConfirm, setDeleteConfirm] = useState<{ type: 'user' | 'folder'; item: User | Node } | null>(null)
  const [movingItem, setMovingItem] = useState<{ type: 'user' | 'folder'; item: User | Node } | null>(null)
  const [isUserNode, setIsUserNode] = useState(false) // Track if current path is a user node
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadContent()
  }, [repo, activeBranch, currentPath])

  async function loadContent() {
    console.log('[Users] loadContent called', { repo, activeBranch, currentPath })
    if (!repo) {
      console.error('[Users] No repo parameter found in URL')
      return
    }
    setLoading(true)
    setIsUserNode(false)

    try {
      // First, check if the current path points to a User node (not a folder)
      if (currentPath !== '/users') {
        try {
          const targetNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, currentPath)
          if (targetNode && targetNode.node_type === 'raisin:User') {
            // Current path is a User node - render editor instead of list
            setIsUserNode(true)
            setLoading(false)
            return
          }
        } catch {
          // Node doesn't exist or can't be loaded - continue with folder loading
        }
      }

      let nodes: Node[]

      // Load nodes from current path
      if (currentPath === '/users') {
        // Load root level of /users folder
        console.log('[Users] Loading root /users folder')
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, '/users')
      } else {
        // Load children of current path
        console.log('[Users] Loading folder at path:', currentPath)
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, currentPath)
      }

      console.log('[Users] Received nodes:', nodes)

      // Separate folders and users
      const folderNodes = nodes.filter(n => n.node_type === 'raisin:AclFolder')
      const userNodes = nodes.filter(n => n.node_type === 'raisin:User')

      console.log('[Users] Filtered folders:', folderNodes.length, 'users:', userNodes.length)

      setFolders(folderNodes)
      setUsers(userNodes.map(nodeToUser))
    } catch (error) {
      console.error('[Users] Failed to load content:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(userId: string) {
    if (!repo) return
    const user = users.find(u => u.user_id === userId)
    if (user) {
      setDeleteConfirm({ type: 'user', item: user })
    }
  }

  async function confirmDelete() {
    if (!repo || !deleteConfirm) return

    try {
      if (deleteConfirm.type === 'user') {
        const user = deleteConfirm.item as User
        await nodesApi.delete(repo, activeBranch, WORKSPACE, `${currentPath}/${user.user_id}`)
      } else {
        const folder = deleteConfirm.item as Node
        await nodesApi.delete(repo, activeBranch, WORKSPACE, folder.path)
      }
      setDeleteConfirm(null)
      loadContent()
    } catch (error) {
      console.error('Failed to delete:', error)
      showError('Delete Failed', `Failed to delete ${deleteConfirm.type}`)
      setDeleteConfirm(null)
    }
  }

  function navigateToUsersPath(path?: string) {
    if (!repo) return
    if (!path || path === '/' || path === basePath) {
      navigate(`/${repo}/${activeBranch}/users`)
      return
    }
    // path is workspace path like '/users/myfolder'
    // Extract relative path for URL
    const relativePath = path.startsWith(basePath)
      ? path.slice(basePath.length)
      : path.startsWith('/') ? path : `/${path}`
    navigate(`/${repo}/${activeBranch}/users${relativePath}`)
  }

  function handleFolderClick(folder: Node) {
    navigateToUsersPath(folder.path)
  }

  function handleNavigate(path: string) {
    if (path === '/' || path === '') {
      navigateToUsersPath(basePath)
      return
    }
    navigateToUsersPath(path)
  }

  async function handleSaveFolder(data: { name: string; description: string; icon: string; color: string }) {
    if (!repo) return

    const properties = {
      description: data.description,
      icon: data.icon,
      color: data.color,
    }

    if (editingFolder) {
      // Update existing folder
      const request: UpdateNodeRequest = {
        properties,
        commit: {
          message: `Update folder: ${data.name}`,
          actor: 'admin',
        },
      }
      await nodesApi.update(repo, activeBranch, WORKSPACE, editingFolder.path, request)
    } else {
      // Create new folder in current path
      const request: CreateNodeRequest = {
        name: data.name,
        node_type: 'raisin:AclFolder',
        properties,
        commit: {
          message: `Create folder: ${data.name}`,
          actor: 'admin',
        },
      }

      if (currentPath === '/users') {
        // Create in root of /users
        await nodesApi.create(repo, activeBranch, WORKSPACE, '/users', request)
      } else {
        // Create in current folder
        await nodesApi.create(repo, activeBranch, WORKSPACE, currentPath, request)
      }
    }

    loadContent()
  }

  function handleCreateFolder() {
    setEditingFolder(undefined)
    setShowFolderDialog(true)
  }

  function handleEditFolder(folder: Node) {
    setEditingFolder(folder)
    setShowFolderDialog(true)
  }

  async function handleDeleteFolder(folder: Node) {
    if (!repo) return
    setDeleteConfirm({ type: 'folder', item: folder })
  }

  function handleMoveFolder(folder: Node) {
    setMovingItem({ type: 'folder', item: folder })
  }

  function handleMoveUser(user: User) {
    setMovingItem({ type: 'user', item: user })
  }

  async function confirmMove(destinationPath: string) {
    if (!repo || !movingItem) return

    try {
      const itemPath = movingItem.type === 'user'
        ? (movingItem.item as User).path
        : (movingItem.item as Node).path
      const itemName = movingItem.type === 'user'
        ? (movingItem.item as User).display_name
        : (movingItem.item as Node).name

      // The move API expects targetPath to be the FULL new path including node name
      // Extract node name from source path and construct full destination
      const nodeName = itemPath.split('/').pop()
      const fullDestination = `${destinationPath}/${nodeName}`

      await nodesApi.move(repo, activeBranch, WORKSPACE, itemPath, {
        destination: fullDestination,
        commit: {
          message: `Move ${movingItem.type} "${itemName}" to ${destinationPath}`,
          actor: 'admin',
        },
      })

      showSuccess('Moved', `${movingItem.type === 'user' ? 'User' : 'Folder'} moved successfully`)
      setMovingItem(null)
      loadContent()
    } catch (error: any) {
      console.error('Failed to move:', error)
      throw new Error(error.message || `Failed to move ${movingItem.type}`)
    }
  }

  // Handle reorder via drag and drop
  const handleReorder = useCallback(async (sourcePath: string, targetPath: string, position: DropPosition) => {
    if (!repo || !position) return

    try {
      await nodesApi.reorder(repo, activeBranch, WORKSPACE, sourcePath, {
        targetPath,
        position,
        commit: {
          message: `Reorder item`,
          actor: 'admin',
        },
      })
      loadContent()
    } catch (error) {
      console.error('Failed to reorder:', error)
      showError('Reorder Failed', 'Failed to reorder item')
    }
  }, [repo, activeBranch])

  // Handle drop into folder (move item into folder)
  const handleDropIntoFolder = useCallback(async (folderPath: string, sourcePath: string, sourceType: string) => {
    if (!repo) return

    try {
      // The move API expects targetPath to be the FULL new path including node name
      // Extract node name from source path and construct full destination
      const nodeName = sourcePath.split('/').pop()
      const fullDestination = `${folderPath}/${nodeName}`

      await nodesApi.move(repo, activeBranch, WORKSPACE, sourcePath, {
        destination: fullDestination,
        commit: {
          message: `Move ${sourceType} into folder`,
          actor: 'admin',
        },
      })
      showSuccess('Moved', `${sourceType === 'user' ? 'User' : 'Folder'} moved successfully`)
      loadContent()
    } catch (error) {
      console.error('Failed to move into folder:', error)
      showError('Move Failed', 'Failed to move item into folder')
    }
  }, [repo, activeBranch])

  // Monitor for drag and drop events
  useEffect(() => {
    return monitorForElements({
      onDrop: ({ source, location }) => {
        const destination = location.current.dropTargets[0]
        if (!destination) return

        const sourceData = source.data as unknown as DragData
        const destData = destination.data as unknown as { path?: string }

        if (!sourceData.path || !destData.path) return
        if (sourceData.path === destData.path) return

        // Get the drop position from the element's drop state
        const destElement = destination.element
        const rect = destElement.getBoundingClientRect()
        const mouseX = location.current.input.clientX
        const midpoint = rect.left + rect.width / 2
        const position: DropPosition = mouseX < midpoint ? 'before' : 'after'

        handleReorder(sourceData.path, destData.path, position)
      },
    })
  }, [handleReorder])

  // Build breadcrumb segments from current path
  const breadcrumbSegments = relativeCurrentPath
    .split('/')
    .filter(Boolean)
    .map((segment, index, array) => ({
      label: segment,
      path: '/' + array.slice(0, index + 1).join('/'),
    }))

  // Define table columns for users
  const userColumns: TableColumn<User>[] = [
    {
      key: 'display_name',
      header: 'Name',
      render: (user) => (
        <div className="flex items-center gap-2">
          <UserIcon className="w-4 h-4 text-primary-400" />
          <span className="text-white font-medium">{user.display_name}</span>
        </div>
      ),
    },
    {
      key: 'user_id',
      header: 'ID',
      render: (user) => <span className="text-zinc-400">{user.user_id}</span>,
    },
    {
      key: 'email',
      header: 'Email',
      render: (user) => <span className="text-zinc-300">{user.email}</span>,
    },
    {
      key: 'roles',
      header: 'Roles',
      render: (user) => (
        <div className="flex flex-wrap gap-1">
          {user.roles?.map((role) => (
            <span key={role} className="px-2 py-0.5 bg-purple-500/20 text-purple-300 text-xs rounded-full">
              {role}
            </span>
          ))}
        </div>
      ),
    },
    {
      key: 'groups',
      header: 'Groups',
      render: (user) => (
        <div className="flex flex-wrap gap-1">
          {user.groups?.map((group) => (
            <span key={group} className="px-2 py-0.5 bg-blue-500/20 text-blue-300 text-xs rounded-full">
              {group}
            </span>
          ))}
        </div>
      ),
    },
  ]

  // If current path is a User node, render the editor instead
  // Only show editor after loading completes and we've confirmed it's a user node
  if (isUserNode && !loading) {
    return <UserEditor />
  }

  return (
    <div className="animate-fade-in">
      {/* Breadcrumb */}
      {breadcrumbSegments.length > 0 && (
        <div className="mb-6">
          <Breadcrumb segments={breadcrumbSegments} onNavigate={handleNavigate} />
        </div>
      )}

      <div className="mb-8 flex flex-col md:flex-row justify-between items-start gap-4">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <h1 className="text-4xl font-bold text-white">Users</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage user accounts and authentication</p>
        </div>
        <div className="flex gap-2 w-full md:w-auto">
          <button
            onClick={handleCreateFolder}
            className="flex-1 md:flex-none flex items-center justify-center gap-2 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors"
          >
            <FolderPlus className="w-5 h-5" />
            <span className="md:inline">New Folder</span>
          </button>
          <Link
            to={`/${repo}/${activeBranch}/users/new?parentPath=${encodeURIComponent(currentPath)}`}
            className="flex-1 md:flex-none flex items-center justify-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Plus className="w-5 h-5" />
            <span className="md:inline">New User</span>
          </Link>
        </div>
      </div>

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading...</div>
      ) : (
        <div className="space-y-8">
          {/* Folders */}
          {folders.length > 0 && (
            <div>
              <h2 className="text-xl font-semibold text-white mb-4">Folders</h2>
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {folders.map((folder) => (
                  <DraggableCardWrapper
                    key={folder.id}
                    id={folder.id}
                    path={folder.path}
                    name={folder.name}
                    type="folder"
                  >
                    <FolderCard
                      folder={folder}
                      onClick={() => handleFolderClick(folder)}
                      onEdit={handleEditFolder}
                      onDelete={handleDeleteFolder}
                      onMove={handleMoveFolder}
                      onDropInto={(sourcePath, sourceType) => handleDropIntoFolder(folder.path, sourcePath, sourceType)}
                    />
                  </DraggableCardWrapper>
                ))}
              </div>
            </div>
          )}

          {/* Users */}
          {users.length === 0 && folders.length === 0 ? (
            <GlassCard>
              <div className="text-center py-12">
                <UserIcon className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
                <h3 className="text-xl font-semibold text-white mb-2">No content yet</h3>
                <p className="text-zinc-400">Create a folder or user to get started</p>
              </div>
            </GlassCard>
          ) : users.length > 0 ? (
            <div>
              <h2 className="text-xl font-semibold text-white mb-4">Users</h2>
              <GlassCard className="overflow-hidden">
                <ItemTable
                  items={users}
                  columns={userColumns}
                  getItemId={(u) => u.id || u.user_id}
                  getItemPath={(u) => u.path}
                  getItemName={(u) => u.display_name}
                  itemType="user"
                  editPath={(u) => `/${repo}/${activeBranch}${u.path}`}
                  onDelete={(u) => handleDelete(u.user_id)}
                  onMove={handleMoveUser}
                  onReorder={(sourcePath, targetPath, position) => handleReorder(sourcePath, targetPath, position)}
                />
              </GlassCard>
            </div>
          ) : null}
        </div>
      )}

      {/* Folder Dialog */}
      {showFolderDialog && (
        <FolderDialog
          folder={editingFolder}
          onClose={() => {
            setShowFolderDialog(false)
            setEditingFolder(undefined)
          }}
          onSave={handleSaveFolder}
        />
      )}

      {/* Delete Confirmation Dialog */}
      <ConfirmDialog
        open={deleteConfirm !== null}
        title={`Delete ${deleteConfirm?.type === 'user' ? 'User' : 'Folder'}`}
        message={
          deleteConfirm?.type === 'user'
            ? `Are you sure you want to delete user "${(deleteConfirm.item as User).display_name}"?`
            : `Are you sure you want to delete folder "${(deleteConfirm?.item as Node)?.name || 'this folder'}"? This will also delete all contents.`
        }
        variant="danger"
        confirmText="Delete"
        onConfirm={confirmDelete}
        onCancel={() => setDeleteConfirm(null)}
      />

      {/* Move to Folder Dialog */}
      {movingItem && repo && (
        <MoveToFolderDialog
          repo={repo}
          branch={activeBranch}
          itemName={movingItem.type === 'user'
            ? (movingItem.item as User).display_name
            : (movingItem.item as Node).name}
          itemType={movingItem.type}
          currentPath={movingItem.type === 'user'
            ? (movingItem.item as User).path
            : (movingItem.item as Node).path}
          basePath={basePath}
          onClose={() => setMovingItem(null)}
          onMove={confirmMove}
        />
      )}

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
