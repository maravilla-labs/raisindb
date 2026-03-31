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
import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest, type NodeRelationships } from '../api/nodes'
import type { DragData, DropPosition } from '../hooks/useDraggableCard'
import EntityCircleEditor from './EntityCircleEditor'

const WORKSPACE = 'raisin:access_control'

interface EntityCircle {
  id?: string
  nodeName: string    // node name (different from display name)
  path: string        // full path within workspace
  name: string        // display name
  circle_type?: string
  primary_contact_id?: string
  primary_contact_name?: string // Resolved display name
  address?: {
    street?: string
    city?: string
    state?: string
    postal_code?: string
    country?: string
  }
  metadata?: Record<string, unknown>
  member_count?: number
  created_at?: string
  updated_at?: string
}

function nodeToEntityCircle(node: Node, memberCount = 0, contactName?: string): EntityCircle {
  return {
    id: node.id,
    nodeName: node.name,
    path: node.path,
    name: node.properties?.name as string,
    circle_type: node.properties?.circle_type as string | undefined,
    primary_contact_id: node.properties?.primary_contact_id as string | undefined,
    primary_contact_name: contactName,
    address: node.properties?.address as EntityCircle['address'] | undefined,
    metadata: node.properties?.metadata as Record<string, unknown> | undefined,
    member_count: memberCount,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

const CIRCLE_TYPE_COLORS: Record<string, string> = {
  family: 'bg-pink-500/20 text-pink-300 border-pink-400/30',
  team: 'bg-blue-500/20 text-blue-300 border-blue-400/30',
  org_unit: 'bg-purple-500/20 text-purple-300 border-purple-400/30',
  department: 'bg-green-500/20 text-green-300 border-green-400/30',
  project: 'bg-amber-500/20 text-amber-300 border-amber-400/30',
  custom: 'bg-zinc-500/20 text-zinc-300 border-zinc-400/30',
}

export default function EntityCircles() {
  const navigate = useNavigate()
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const basePath = '/circles'
  const wildcardPath = pathParam ? `/${pathParam}` : ''
  const currentPath = wildcardPath
    ? wildcardPath.startsWith(basePath)
      ? wildcardPath
      : `${basePath}${wildcardPath}`
    : basePath
  const relativeCurrentPath = currentPath === basePath ? '' : currentPath.slice(basePath.length)

  const [folders, setFolders] = useState<Node[]>([])
  const [circles, setCircles] = useState<EntityCircle[]>([])
  const [loading, setLoading] = useState(true)
  const [showFolderDialog, setShowFolderDialog] = useState(false)
  const [editingFolder, setEditingFolder] = useState<Node | undefined>(undefined)
  const [deleteConfirm, setDeleteConfirm] = useState<{ type: 'circle' | 'folder'; item: EntityCircle | Node } | null>(null)
  const [movingItem, setMovingItem] = useState<{ type: 'circle' | 'folder'; item: EntityCircle | Node } | null>(null)
  const [isCircleNode, setIsCircleNode] = useState(false) // Track if current path is a circle node
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadContent()
  }, [repo, activeBranch, currentPath])

  async function loadContent() {
    if (!repo) return
    setLoading(true)
    setIsCircleNode(false)

    try {
      // First, check if the current path points to an EntityCircle node (not a folder)
      if (currentPath !== '/circles') {
        try {
          const targetNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, currentPath)
          if (targetNode && targetNode.node_type === 'raisin:EntityCircle') {
            // Current path is an EntityCircle node - render editor instead of list
            setIsCircleNode(true)
            setLoading(false)
            return
          }
        } catch {
          // Node doesn't exist or can't be loaded - continue with folder loading
        }
      }

      let nodes: Node[]

      // Load nodes from current path
      if (currentPath === '/circles') {
        // Load root level of /circles folder
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, '/circles')
      } else {
        // Load children of current path
        nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, currentPath)
      }

      // Separate folders and circles
      const folderNodes = nodes.filter(n => n.node_type === 'raisin:AclFolder')
      const circleNodes = nodes.filter(n => n.node_type === 'raisin:EntityCircle')

      // Load member counts and primary contact names for circles
      const circlesWithMetadata = await Promise.all(
        circleNodes.map(async (node) => {
          let memberCount = 0
          let contactName: string | undefined

          try {
            // Get relationships to count members
            const relationships: NodeRelationships = await nodesApi.getRelationships(repo, activeBranch, WORKSPACE, node.path)
            // Count incoming MEMBER_OF relationships
            memberCount = relationships.incoming.filter(rel => rel.relation_type === 'MEMBER_OF').length
          } catch (error) {
            console.error('Failed to load relationships for circle:', node.path, error)
          }

          // Try to resolve primary contact display name
          const contactId = node.properties?.primary_contact_id as string | undefined
          if (contactId) {
            try {
              const contactNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, `/users/${contactId}`)
              contactName = contactNode.properties?.display_name as string || contactId
            } catch {
              contactName = contactId // Fallback to ID if user not found
            }
          }

          return nodeToEntityCircle(node, memberCount, contactName)
        })
      )

      setFolders(folderNodes)
      setCircles(circlesWithMetadata)
    } catch (error) {
      console.error('Failed to load content:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(circleName: string) {
    if (!repo) return
    const circle = circles.find(c => c.nodeName === circleName)
    if (circle) {
      setDeleteConfirm({ type: 'circle', item: circle })
    }
  }

  async function confirmDelete() {
    if (!repo || !deleteConfirm) return

    try {
      if (deleteConfirm.type === 'circle') {
        const circle = deleteConfirm.item as EntityCircle
        await nodesApi.delete(repo, activeBranch, WORKSPACE, circle.path)
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

  function navigateToCirclesPath(path?: string) {
    if (!repo) return
    if (!path || path === '/' || path === basePath) {
      navigate(`/${repo}/${activeBranch}/circles`)
      return
    }
    // path is workspace path like '/circles/myfolder'
    // Extract relative path for URL
    const relativePath = path.startsWith(basePath)
      ? path.slice(basePath.length)
      : path.startsWith('/') ? path : `/${path}`
    navigate(`/${repo}/${activeBranch}/circles${relativePath}`)
  }

  function handleFolderClick(folder: Node) {
    navigateToCirclesPath(folder.path)
  }

  function handleNavigate(path: string) {
    if (path === '/' || path === '') {
      navigateToCirclesPath(basePath)
      return
    }
    navigateToCirclesPath(path)
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

      if (currentPath === '/circles') {
        await nodesApi.create(repo, activeBranch, WORKSPACE, '/circles', request)
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

  function handleMoveCircle(circle: EntityCircle) {
    setMovingItem({ type: 'circle', item: circle })
  }

  async function confirmMove(destinationPath: string) {
    if (!repo || !movingItem) return

    try {
      const itemPath = movingItem.type === 'circle'
        ? (movingItem.item as EntityCircle).path
        : (movingItem.item as Node).path
      const itemName = movingItem.type === 'circle'
        ? (movingItem.item as EntityCircle).name
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

      showSuccess('Moved', `${movingItem.type === 'circle' ? 'Circle' : 'Folder'} moved successfully`)
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
      showSuccess('Moved', `${sourceType === 'circle' ? 'Circle' : 'Folder'} moved successfully`)
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

  // Define table columns for circles
  const circleColumns: TableColumn<EntityCircle>[] = [
    {
      key: 'name',
      header: 'Circle Name',
      render: (circle) => (
        <div className="flex items-center gap-2">
          <Users className="w-4 h-4 text-primary-400" />
          <span className="text-white font-medium">{circle.name}</span>
        </div>
      ),
    },
    {
      key: 'circle_type',
      header: 'Type',
      render: (circle) => circle.circle_type ? (
        <span className={`px-2 py-0.5 text-xs rounded-full border ${CIRCLE_TYPE_COLORS[circle.circle_type] || CIRCLE_TYPE_COLORS.custom}`}>
          {circle.circle_type}
        </span>
      ) : (
        <span className="text-zinc-500 text-sm">-</span>
      ),
    },
    {
      key: 'members',
      header: 'Members',
      render: (circle) => (
        <span className="text-zinc-300">{circle.member_count || 0}</span>
      ),
    },
    {
      key: 'primary_contact',
      header: 'Primary Contact',
      render: (circle) => (
        <span className="text-zinc-300 text-sm">
          {circle.primary_contact_name || circle.primary_contact_id || '-'}
        </span>
      ),
    },
    {
      key: 'location',
      header: 'Location',
      render: (circle) => {
        const parts = []
        if (circle.address?.city) parts.push(circle.address.city)
        if (circle.address?.state) parts.push(circle.address.state)
        if (circle.address?.country) parts.push(circle.address.country)
        return (
          <span className="text-zinc-400 text-sm">
            {parts.length > 0 ? parts.join(', ') : '-'}
          </span>
        )
      },
    },
  ]

  // If current path is an EntityCircle node, render the editor instead
  // Only show editor after loading completes and we've confirmed it's a circle node
  if (isCircleNode && !loading) {
    return <EntityCircleEditor />
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
            <h1 className="text-4xl font-bold text-white">Entity Circles</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage entity circles for stewardship and access control</p>
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
            to={`/${repo}/${activeBranch}/circles/new?parentPath=${encodeURIComponent(currentPath)}`}
            className="flex-1 md:flex-none flex items-center justify-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Plus className="w-5 h-5" />
            <span className="md:inline">New Circle</span>
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

          {/* Entity Circles */}
          {circles.length === 0 && folders.length === 0 ? (
            <GlassCard>
              <div className="text-center py-12">
                <Users className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
                <h3 className="text-xl font-semibold text-white mb-2">No circles yet</h3>
                <p className="text-zinc-400">Create a folder or entity circle to get started</p>
              </div>
            </GlassCard>
          ) : circles.length > 0 ? (
            <div>
              <h2 className="text-xl font-semibold text-white mb-4">Circles</h2>
              <GlassCard className="overflow-hidden">
                <ItemTable
                  items={circles}
                  columns={circleColumns}
                  getItemId={(c) => c.id || c.nodeName}
                  getItemPath={(c) => c.path}
                  getItemName={(c) => c.name}
                  itemType="circle"
                  editPath={(c) => `/${repo}/${activeBranch}${c.path}`}
                  onDelete={(c) => handleDelete(c.nodeName)}
                  onMove={handleMoveCircle}
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
        title={`Delete ${deleteConfirm?.type === 'circle' ? 'Circle' : 'Folder'}`}
        message={
          deleteConfirm?.type === 'circle'
            ? `Are you sure you want to delete circle "${(deleteConfirm.item as EntityCircle).name}"?`
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
          itemName={movingItem.type === 'circle'
            ? (movingItem.item as EntityCircle).name
            : (movingItem.item as Node).name}
          itemType={movingItem.type}
          currentPath={movingItem.type === 'circle'
            ? (movingItem.item as EntityCircle).path
            : (movingItem.item as Node).path}
          basePath="/circles"
          onClose={() => setMovingItem(null)}
          onMove={confirmMove}
        />
      )}

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
