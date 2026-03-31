import { useEffect, useState, useCallback } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { Shield, Plus, FolderPlus } from 'lucide-react'
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
import RoleEditor from './RoleEditor'

const WORKSPACE = 'raisin:access_control'

interface Role {
  id?: string
  nodeName: string    // node name (different from display name)
  path: string        // full path within workspace
  role_id: string
  name: string        // display name
  description?: string
  permissions?: string[]
  inherits?: string[]
  created_at?: string
  updated_at?: string
}

function nodeToRole(node: Node): Role {
  return {
    id: node.id,
    nodeName: node.name,
    path: node.path,
    role_id: node.properties?.role_id as string,
    name: node.properties?.name as string,
    description: node.properties?.description as string | undefined,
    permissions: node.properties?.permissions as string[] | undefined,
    inherits: node.properties?.inherits as string[] | undefined,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

export default function Roles() {
  const navigate = useNavigate()
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const basePath = '/roles'
  const wildcardPath = pathParam ? `/${pathParam}` : ''
  const currentPath = wildcardPath
    ? wildcardPath.startsWith(basePath)
      ? wildcardPath
      : `${basePath}${wildcardPath}`
    : basePath
  const relativeCurrentPath = currentPath === basePath ? '' : currentPath.slice(basePath.length)

  const [folders, setFolders] = useState<Node[]>([])
  const [roles, setRoles] = useState<Role[]>([])
  const [loading, setLoading] = useState(true)
  const [showFolderDialog, setShowFolderDialog] = useState(false)
  const [editingFolder, setEditingFolder] = useState<Node | undefined>(undefined)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const [movingItem, setMovingItem] = useState<{ type: 'role' | 'folder'; item: Role | Node } | null>(null)
  const [isRoleNode, setIsRoleNode] = useState(false) // Track if current path is a role node
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadContent()
  }, [repo, activeBranch, currentPath])

  async function loadContent() {
    if (!repo) return
    setLoading(true)
    setIsRoleNode(false)

    try {
      // First, check if the current path points to a Role node (not a folder)
      if (currentPath !== basePath) {
        try {
          const targetNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, currentPath)
          if (targetNode && targetNode.node_type === 'raisin:Role') {
            // Current path is a Role node - render editor instead of list
            setIsRoleNode(true)
            setLoading(false)
            return
          }
        } catch (e) {
          // Node doesn't exist or can't be loaded - continue with folder loading
        }
      }

      let nodes: Node[]

      // Load nodes from current path
      if (currentPath === '/roles') {
        // Load root level of /roles folder
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, '/roles')
      } else {
        // Load children of current path
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, currentPath)
      }

      // Separate folders and roles
      const folderNodes = nodes.filter(n => n.node_type === 'raisin:AclFolder')
      const roleNodes = nodes.filter(n => n.node_type === 'raisin:Role')

      setFolders(folderNodes)
      setRoles(roleNodes.map(nodeToRole))
    } catch (error) {
      console.error('Failed to load content:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(roleId: string) {
    if (!repo) return
    setDeleteConfirm({
      message: `Are you sure you want to delete role "${roleId}"?`,
      onConfirm: async () => {
        try {
          await nodesApi.delete(repo, activeBranch, WORKSPACE, `${currentPath}/${roleId}`)
          loadContent()
          showSuccess('Deleted', 'Role deleted successfully')
        } catch (error) {
          console.error('Failed to delete role:', error)
          showError('Delete Failed', 'Failed to delete role')
        }
      }
    })
  }

  function navigateToRolesPath(path?: string) {
    if (!repo) return
    if (!path || path === '/' || path === basePath) {
      navigate(`/${repo}/${activeBranch}/roles`)
      return
    }
    // path is workspace path like '/roles/myfolder'
    // Extract relative path for URL
    const relativePath = path.startsWith(basePath)
      ? path.slice(basePath.length)
      : path
    navigate(`/${repo}/${activeBranch}/roles${relativePath}`)
  }

  function handleFolderClick(folder: Node) {
    navigateToRolesPath(folder.path)
  }

  function handleNavigate(path: string) {
    if (path === '/' || path === '') {
      navigateToRolesPath(basePath)
      return
    }
    navigateToRolesPath(path)
  }

  async function handleSaveFolder(data: { name: string; description: string; icon: string; color: string }) {
    if (!repo) return

    const properties = {
      description: data.description,
      icon: data.icon,
      color: data.color,
    }

    if (editingFolder) {
      const request: UpdateNodeRequest = {
        properties,
        commit: {
          message: `Update folder: ${data.name}`,
          actor: 'admin',
        },
      }
      await nodesApi.update(repo, activeBranch, WORKSPACE, editingFolder.path, request)
    } else {
      const request: CreateNodeRequest = {
        name: data.name,
        node_type: 'raisin:AclFolder',
        properties,
        commit: {
          message: `Create folder: ${data.name}`,
          actor: 'admin',
        },
      }

      if (currentPath === '/roles') {
        await nodesApi.create(repo, activeBranch, WORKSPACE, '/roles', request)
      } else {
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
    setDeleteConfirm({
      message: `Are you sure you want to delete folder "${folder.name}"? This will also delete all contents.`,
      onConfirm: async () => {
        try {
          await nodesApi.delete(repo, activeBranch, WORKSPACE, folder.path)
          loadContent()
          showSuccess('Deleted', 'Folder deleted successfully')
        } catch (error) {
          console.error('Failed to delete folder:', error)
          showError('Delete Failed', 'Failed to delete folder')
        }
      }
    })
  }

  function handleMoveFolder(folder: Node) {
    setMovingItem({ type: 'folder', item: folder })
  }

  function handleMoveRole(role: Role) {
    setMovingItem({ type: 'role', item: role })
  }

  async function confirmMove(destinationPath: string) {
    if (!repo || !movingItem) return

    try {
      const itemPath = movingItem.type === 'role'
        ? (movingItem.item as Role).path
        : (movingItem.item as Node).path
      const itemName = movingItem.type === 'role'
        ? (movingItem.item as Role).name
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

      showSuccess('Moved', `${movingItem.type === 'role' ? 'Role' : 'Folder'} moved successfully`)
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
      showSuccess('Moved', `${sourceType === 'role' ? 'Role' : 'Folder'} moved successfully`)
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

  // Define table columns for roles
  const roleColumns: TableColumn<Role>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (role) => (
        <div className="flex items-center gap-2">
          <Shield className="w-4 h-4 text-primary-400" />
          <span className="text-white font-medium">{role.name}</span>
        </div>
      ),
    },
    {
      key: 'role_id',
      header: 'ID',
      render: (role) => <span className="text-zinc-400">{role.role_id}</span>,
    },
    {
      key: 'description',
      header: 'Description',
      render: (role) => <span className="text-zinc-300 text-sm">{role.description || '-'}</span>,
    },
    {
      key: 'permissions',
      header: 'Permissions',
      render: (role) => (
        <span className="text-primary-300">{role.permissions?.length || 0}</span>
      ),
    },
    {
      key: 'inherits',
      header: 'Inherits',
      render: (role) => (
        <div className="flex flex-wrap gap-1">
          {role.inherits?.map((inheritedRole) => (
            <span key={inheritedRole} className="px-2 py-0.5 bg-indigo-500/20 text-indigo-300 text-xs rounded-full">
              {inheritedRole}
            </span>
          ))}
        </div>
      ),
    },
  ]

  // If current path is a Role node, render the editor instead
  // Only show editor after loading completes and we've confirmed it's a role node
  if (isRoleNode && !loading) {
    return <RoleEditor />
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
            <h1 className="text-4xl font-bold text-white">Roles</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage roles and permissions</p>
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
            to={`/${repo}/${activeBranch}/roles/new${currentPath !== '/roles' ? `?parentPath=${encodeURIComponent(currentPath)}` : ''}`}
            className="flex-1 md:flex-none flex items-center justify-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Plus className="w-5 h-5" />
            <span className="md:inline">New Role</span>
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

          {/* Roles */}
          {roles.length === 0 && folders.length === 0 ? (
            <GlassCard>
              <div className="text-center py-12">
                <Shield className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
                <h3 className="text-xl font-semibold text-white mb-2">No content yet</h3>
                <p className="text-zinc-400">Create a folder or role to get started</p>
              </div>
            </GlassCard>
          ) : roles.length > 0 ? (
            <div>
              <h2 className="text-xl font-semibold text-white mb-4">Roles</h2>
              <GlassCard className="overflow-hidden">
                <ItemTable
                  items={roles}
                  columns={roleColumns}
                  getItemId={(r) => r.id || r.role_id}
                  getItemPath={(r) => r.path}
                  getItemName={(r) => r.name}
                  itemType="role"
                  editPath={(r) => `/${repo}/${activeBranch}${r.path}`}
                  onDelete={(r) => handleDelete(r.role_id)}
                  onMove={handleMoveRole}
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

      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Confirm Delete"
        message={deleteConfirm?.message || ''}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          deleteConfirm?.onConfirm()
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />

      {/* Move to Folder Dialog */}
      {movingItem && repo && (
        <MoveToFolderDialog
          repo={repo}
          branch={activeBranch}
          itemName={movingItem.type === 'role'
            ? (movingItem.item as Role).name
            : (movingItem.item as Node).name}
          itemType={movingItem.type}
          currentPath={movingItem.type === 'role'
            ? (movingItem.item as Role).path
            : (movingItem.item as Node).path}
          basePath="/roles"
          onClose={() => setMovingItem(null)}
          onMove={confirmMove}
        />
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
