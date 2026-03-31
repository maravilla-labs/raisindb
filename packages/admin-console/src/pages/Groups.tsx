import { useEffect, useState, useCallback } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { Users, Plus, FolderPlus } from 'lucide-react'
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
import GroupEditor from './GroupEditor'

const WORKSPACE = 'raisin:access_control'

interface Group {
  id?: string
  nodeName: string    // node name (different from display name)
  path: string        // full path within workspace
  group_id: string
  name: string        // display name
  description?: string
  roles?: string[]
  created_at?: string
  updated_at?: string
}

function nodeToGroup(node: Node): Group {
  return {
    id: node.id,
    nodeName: node.name,
    path: node.path,
    group_id: node.properties?.group_id as string,
    name: node.properties?.name as string,
    description: node.properties?.description as string | undefined,
    roles: node.properties?.roles as string[] | undefined,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

export default function Groups() {
  const navigate = useNavigate()
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const basePath = '/groups'
  const wildcardPath = pathParam ? `/${pathParam}` : ''
  const currentPath = wildcardPath
    ? wildcardPath.startsWith(basePath)
      ? wildcardPath
      : `${basePath}${wildcardPath}`
    : basePath
  const relativeCurrentPath = currentPath === basePath ? '' : currentPath.slice(basePath.length)

  const [folders, setFolders] = useState<Node[]>([])
  const [groups, setGroups] = useState<Group[]>([])
  const [loading, setLoading] = useState(true)
  const [showFolderDialog, setShowFolderDialog] = useState(false)
  const [editingFolder, setEditingFolder] = useState<Node | undefined>(undefined)
  const [deleteConfirm, setDeleteConfirm] = useState<{ type: 'group' | 'folder'; item: Group | Node } | null>(null)
  const [movingItem, setMovingItem] = useState<{ type: 'group' | 'folder'; item: Group | Node } | null>(null)
  const [isGroupNode, setIsGroupNode] = useState(false) // Track if current path is a group node
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadContent()
  }, [repo, activeBranch, currentPath])

  async function loadContent() {
    if (!repo) return
    setLoading(true)
    setIsGroupNode(false)

    try {
      // First, check if the current path points to a Group node (not a folder)
      if (currentPath !== '/groups') {
        try {
          const targetNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, currentPath)
          if (targetNode && targetNode.node_type === 'raisin:Group') {
            // Current path is a Group node - render editor instead of list
            setIsGroupNode(true)
            setLoading(false)
            return
          }
        } catch {
          // Node doesn't exist or can't be loaded - continue with folder loading
        }
      }

      let nodes: Node[]

      // Load nodes from current path
      if (currentPath === '/groups') {
        // Load root level of /groups folder
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, '/groups')
      } else {
        // Load children of current path
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, currentPath)
      }

      // Separate folders and groups
      const folderNodes = nodes.filter(n => n.node_type === 'raisin:AclFolder')
      const groupNodes = nodes.filter(n => n.node_type === 'raisin:Group')

      setFolders(folderNodes)
      setGroups(groupNodes.map(nodeToGroup))
    } catch (error) {
      console.error('Failed to load content:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(groupId: string) {
    if (!repo) return
    const group = groups.find(g => g.group_id === groupId)
    if (group) {
      setDeleteConfirm({ type: 'group', item: group })
    }
  }

  async function confirmDelete() {
    if (!repo || !deleteConfirm) return

    try {
      if (deleteConfirm.type === 'group') {
        const group = deleteConfirm.item as Group
        await nodesApi.delete(repo, activeBranch, WORKSPACE, `${currentPath}/${group.group_id}`)
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

    function navigateToGroupsPath(path?: string) {
    if (!repo) return
    if (!path || path === '/' || path === basePath) {
      navigate(`/${repo}/${activeBranch}/groups`)
      return
    }
    // path is workspace path like '/groups/myfolder'
    // Extract relative path for URL
    const relativePath = path.startsWith(basePath)
      ? path.slice(basePath.length)
      : path.startsWith('/') ? path : `/${path}`
    navigate(`/${repo}/${activeBranch}/groups${relativePath}`)
  }

  function handleFolderClick(folder: Node) {
    navigateToGroupsPath(folder.path)
  }

  function handleNavigate(path: string) {
    if (path === '/' || path === '') {
      navigateToGroupsPath(basePath)
      return
    }
    navigateToGroupsPath(path)
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

      if (currentPath === '/groups') {
        await nodesApi.create(repo, activeBranch, WORKSPACE, '/groups', request)
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
    setDeleteConfirm({ type: 'folder', item: folder })
  }

  function handleMoveFolder(folder: Node) {
    setMovingItem({ type: 'folder', item: folder })
  }

  function handleMoveGroup(group: Group) {
    setMovingItem({ type: 'group', item: group })
  }

  async function confirmMove(destinationPath: string) {
    if (!repo || !movingItem) return

    try {
      const itemPath = movingItem.type === 'group'
        ? (movingItem.item as Group).path
        : (movingItem.item as Node).path
      const itemName = movingItem.type === 'group'
        ? (movingItem.item as Group).name
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

      showSuccess('Moved', `${movingItem.type === 'group' ? 'Group' : 'Folder'} moved successfully`)
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
      showSuccess('Moved', `${sourceType === 'group' ? 'Group' : 'Folder'} moved successfully`)
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

  // Define table columns for groups
  const groupColumns: TableColumn<Group>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (group) => (
        <div className="flex items-center gap-2">
          <Users className="w-4 h-4 text-primary-400" />
          <span className="text-white font-medium">{group.name}</span>
        </div>
      ),
    },
    {
      key: 'group_id',
      header: 'ID',
      render: (group) => <span className="text-zinc-400">{group.group_id}</span>,
    },
    {
      key: 'description',
      header: 'Description',
      render: (group) => <span className="text-zinc-300 text-sm">{group.description || '-'}</span>,
    },
    {
      key: 'roles',
      header: 'Roles',
      render: (group) => (
        <div className="flex flex-wrap gap-1">
          {group.roles?.map((role) => (
            <span key={role} className="px-2 py-0.5 bg-purple-500/20 text-purple-300 text-xs rounded-full">
              {role}
            </span>
          ))}
        </div>
      ),
    },
  ]

  // If current path is a Group node, render the editor instead
  // Only show editor after loading completes and we've confirmed it's a group node
  if (isGroupNode && !loading) {
    return <GroupEditor />
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
            <h1 className="text-4xl font-bold text-white">Groups</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage user groups and role assignments</p>
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
            to={`/${repo}/${activeBranch}/groups/new?parentPath=${encodeURIComponent(currentPath)}`}
            className="flex-1 md:flex-none flex items-center justify-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Plus className="w-5 h-5" />
            <span className="md:inline">New Group</span>
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

          {/* Groups */}
          {groups.length === 0 && folders.length === 0 ? (
            <GlassCard>
              <div className="text-center py-12">
                <Users className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
                <h3 className="text-xl font-semibold text-white mb-2">No content yet</h3>
                <p className="text-zinc-400">Create a folder or group to get started</p>
              </div>
            </GlassCard>
          ) : groups.length > 0 ? (
            <div>
              <h2 className="text-xl font-semibold text-white mb-4">Groups</h2>
              <GlassCard className="overflow-hidden">
                <ItemTable
                  items={groups}
                  columns={groupColumns}
                  getItemId={(g) => g.id || g.group_id}
                  getItemPath={(g) => g.path}
                  getItemName={(g) => g.name}
                  itemType="group"
                  editPath={(g) => `/${repo}/${activeBranch}${g.path}`}
                  onDelete={(g) => handleDelete(g.group_id)}
                  onMove={handleMoveGroup}
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
        title={`Delete ${deleteConfirm?.type === 'group' ? 'Group' : 'Folder'}`}
        message={
          deleteConfirm?.type === 'group'
            ? `Are you sure you want to delete group "${(deleteConfirm.item as Group).name}"?`
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
          itemName={movingItem.type === 'group'
            ? (movingItem.item as Group).name
            : (movingItem.item as Node).name}
          itemType={movingItem.type}
          currentPath={movingItem.type === 'group'
            ? (movingItem.item as Group).path
            : (movingItem.item as Node).path}
          basePath="/groups"
          onClose={() => setMovingItem(null)}
          onMove={confirmMove}
        />
      )}

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
