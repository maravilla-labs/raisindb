import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { ArrowLeft, Settings, FolderTree, Info, Edit, Save, Trash2, Plus, FileEdit, Wrench } from 'lucide-react'
import { useRepositoryContext } from '../hooks/useRepositoryContext'
import * as yaml from 'js-yaml'
import GlassCard from '../components/GlassCard'
import Tabs, { type Tab } from '../components/Tabs'
import TreeView from '../components/TreeView'
import NodeEditor from '../components/NodeEditor'
import ConfirmDialog from '../components/ConfirmDialog'
import YamlEditor from '../components/YamlEditor'
import CopyNodeModal from '../components/CopyNodeModal'
import MoveNodeModal from '../components/MoveNodeModal'
import CreateNodeDialog from '../components/CreateNodeDialog'
import WorkspaceConfigEditor from '../components/WorkspaceConfigEditor'
import { workspacesApi, type Workspace } from '../api/workspaces'
import { nodesApi, type Node, type CreateNodeRequest } from '../api/nodes'
import { useToast, ToastContainer } from '../components/Toast'

const tabs: Tab[] = [
  { id: 'overview', label: 'Overview', icon: Info },
  { id: 'content', label: 'Content', icon: FolderTree },
  { id: 'config', label: 'Config', icon: Wrench },
  { id: 'settings', label: 'Settings', icon: Settings },
]

export default function WorkspaceDetail() {
  const { repo, branch, workspace: workspaceName } = useRepositoryContext()
  const [workspace, setWorkspace] = useState<Workspace | null>(null)
  const [nodes, setNodes] = useState<Node[]>([])
  const [activeTab, setActiveTab] = useState('overview')
  const [loading, setLoading] = useState(true)
  const [editing, setEditing] = useState(false)
  const [yamlContent, setYamlContent] = useState('')
  const [selectedNode, setSelectedNode] = useState<Node | null>(null)
  const [showNodeEditor, setShowNodeEditor] = useState(false)
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [showCreateNode, setShowCreateNode] = useState(false)
  const [createNodeParent, setCreateNodeParent] = useState<Node | null>(null)
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set())
  const [showCopyModal, setShowCopyModal] = useState(false)
  const [showMoveModal, setShowMoveModal] = useState(false)
  const [nodeToCopy, setNodeToCopy] = useState<Node | null>(null)
  const [nodeToMove, setNodeToMove] = useState<Node | null>(null)
  const [deleteNodeConfirm, setDeleteNodeConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadWorkspace()
    loadNodes()
  }, [repo, branch, workspaceName])

  async function loadWorkspace() {
    if (!repo || !workspaceName) return
    try {
      const data = await workspacesApi.get(repo, workspaceName)
      setWorkspace(data)
      setYamlContent(yaml.dump(data, { indent: 2 }))
    } catch (error) {
      console.error('Failed to load workspace:', error)
    } finally {
      setLoading(false)
    }
  }

  async function loadNodes() {
    try {
      const data = await nodesApi.listRoot(repo, branch, workspaceName)
      setNodes(data)
    } catch (error) {
      console.error('Failed to load nodes:', error)
    }
  }

  async function handleSaveWorkspace() {
    if (!repo || !workspaceName) return
    try {
      const parsed = yaml.load(yamlContent) as Workspace
      await workspacesApi.update(repo, workspaceName, parsed)
      setWorkspace(parsed)
      setEditing(false)
      showSuccess('Success', 'Workspace saved successfully')
    } catch (error) {
      console.error('Failed to save workspace:', error)
      showError('Error', 'Failed to save workspace')
    }
  }

  async function handleDeleteWorkspace() {
    // TODO: Implement workspace delete endpoint
    showError('Not Implemented', 'Workspace deletion not yet implemented')
    setShowDeleteConfirm(false)
  }

  async function handleCreateNode(nodeData: CreateNodeRequest) {
    try {
      if (createNodeParent) {
        // Create child node
        await nodesApi.create(repo, branch, workspaceName, createNodeParent.path, nodeData)
      } else {
        // Create root node
        await nodesApi.createRoot(repo, branch, workspaceName, nodeData)
      }
      setShowCreateNode(false)
      setCreateNodeParent(null)
      loadNodes()
    } catch (error) {
      console.error('Failed to create node:', error)
      showError('Error', 'Failed to create node')
    }
  }

  async function loadNodeChildren(node: Node) {
    try {
      // If children already loaded, just expand
      if (node.children && node.children.length > 0 && typeof node.children[0] === 'object') {
        setExpandedNodes(prev => new Set(prev).add(node.id))
        return
      }

      // Fetch children with level=1 (get children + their immediate children)
      const childDetails = await nodesApi.listChildren(repo, branch, workspaceName, node.path)

      // Update the nodes tree with children
      const updateNodeInTree = (nodes: Node[]): Node[] => {
        return nodes.map(n => {
          if (n.id === node.id) {
            return { ...n, children: childDetails }
          }
          if (n.children) {
            return { ...n, children: updateNodeInTree(n.children) }
          }
          return n
        })
      }

      setNodes(updateNodeInTree(nodes))
      setExpandedNodes(prev => new Set(prev).add(node.id))
    } catch (error) {
      console.error('Failed to load children:', error)
    }
  }

  function handleNodeExpand(node: Node) {
    if (expandedNodes.has(node.id)) {
      // Collapse
      setExpandedNodes(prev => {
        const next = new Set(prev)
        next.delete(node.id)
        return next
      })
    } else {
      // Expand - loadNodeChildren handles checking if already loaded
      loadNodeChildren(node)
    }
  }

  async function handleNodeUpdate(node: Partial<Node>) {
    if (!selectedNode) return
    try {
      await nodesApi.update(repo, branch, workspaceName, selectedNode.path, node)
      loadNodes()
      setShowNodeEditor(false)
    } catch (error) {
      console.error('Failed to save node:', error)
      throw error
    }
  }

  async function handleDeleteNode(node: Node) {
    setDeleteNodeConfirm({
      message: `Delete "${node.name}"? This cannot be undone.`,
      onConfirm: async () => {
        try {
          await nodesApi.delete(repo, branch, workspaceName, node.path)
          loadNodes()
        } catch (error) {
          console.error('Failed to delete node:', error)
          showError('Error', 'Failed to delete node')
        }
      }
    })
  }

  async function handleCopy(destination: string, newName?: string) {
    if (!nodeToCopy) return
    try {
      await nodesApi.copy(repo, branch, workspaceName, nodeToCopy.path, {
        destination,
        name: newName,
        commit: {
          message: `Copy ${nodeToCopy.name} to ${destination}`,
          actor: 'user' // TODO: Get actual user from auth context
        }
      })
      loadNodes()
      setShowCopyModal(false)
      setNodeToCopy(null)
    } catch (error) {
      console.error('Failed to copy node:', error)
      throw error
    }
  }

  async function handleMove(destination: string) {
    if (!nodeToMove) return
    try {
      await nodesApi.move(repo, branch, workspaceName, nodeToMove.path, {
        destination,
        commit: {
          message: `Move ${nodeToMove.name} from ${nodeToMove.path} to ${destination}`,
          actor: 'user' // TODO: Get actual user from auth context
        }
      })
      loadNodes()
      setShowMoveModal(false)
      setNodeToMove(null)
    } catch (error) {
      console.error('Failed to move node:', error)
      throw error
    }
  }

  async function handlePublish(node: Node) {
    try {
      await nodesApi.publish(repo, branch, workspaceName, node.path)
      loadNodes()
    } catch (error) {
      console.error('Failed to publish node:', error)
      showError('Error', 'Failed to publish node')
    }
  }

  async function handleUnpublish(node: Node) {
    try {
      await nodesApi.unpublish(repo, branch, workspaceName, node.path)
      loadNodes()
    } catch (error) {
      console.error('Failed to unpublish node:', error)
      showError('Error', 'Failed to unpublish node')
    }
  }

  if (loading || !workspace) {
    return <div className="text-center text-zinc-400 py-12">Loading...</div>
  }

  return (
    <div className="animate-fade-in">
      <div className="mb-8">
        <Link
          to={`/${repo}/workspaces`}
          className="inline-flex items-center gap-2 text-primary-400 hover:text-primary-300 mb-4"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Workspaces
        </Link>
        <div className="flex justify-between items-start">
          <div>
            <h1 className="text-4xl font-bold text-white mb-2">{workspace.name}</h1>
            <p className="text-zinc-400">{workspace.description || 'No description'}</p>
          </div>
          {activeTab === 'overview' && !editing && (
            <button
              onClick={() => setEditing(true)}
              className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
            >
              <Edit className="w-4 h-4" />
              Edit
            </button>
          )}
        </div>
      </div>

      <GlassCard>
        <Tabs tabs={tabs} activeTab={activeTab} onChange={setActiveTab}>
          {/* Overview Tab */}
          {activeTab === 'overview' && (
            <div className="space-y-6">
              {editing ? (
                <>
                  <div className="p-4 bg-secondary-500/10 border border-secondary-400/30 rounded-lg text-sm text-secondary-300">
                    Edit workspace properties below. Click Save to apply changes.
                  </div>
                  <div className="grid grid-cols-2 gap-6">
                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-2">Name</label>
                      <p className="text-white bg-white/5 px-4 py-2 rounded-lg">{workspace.name}</p>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-2">Description</label>
                      <input
                        type="text"
                        value={workspace.description || ''}
                        onChange={(e) => {
                          const updated = { ...workspace, description: e.target.value }
                          setWorkspace(updated)
                          setYamlContent(yaml.dump(updated, { indent: 2 }))
                        }}
                        className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                      />
                    </div>
                  </div>
                  <div className="flex gap-3">
                    <button
                      onClick={handleSaveWorkspace}
                      className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                    >
                      <Save className="w-4 h-4" />
                      Save
                    </button>
                    <button
                      onClick={() => {
                        setEditing(false)
                        setYamlContent(yaml.dump(workspace, { indent: 2 }))
                      }}
                      className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
                    >
                      Cancel
                    </button>
                    <button
                      onClick={() => setShowDeleteConfirm(true)}
                      className="ml-auto flex items-center gap-2 px-4 py-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors"
                    >
                      <Trash2 className="w-4 h-4" />
                      Delete Workspace
                    </button>
                  </div>
                </>
              ) : (
                <div className="grid grid-cols-2 gap-6">
                  <div>
                    <label className="block text-sm font-medium text-zinc-400 mb-1">Name</label>
                    <p className="text-white">{workspace.name}</p>
                  </div>
                  <div>
                    <label className="block text-sm font-medium text-zinc-400 mb-1">Description</label>
                    <p className="text-white">{workspace.description || 'N/A'}</p>
                  </div>
                  {workspace.allowed_node_types && workspace.allowed_node_types.length > 0 && (
                    <div className="col-span-2">
                      <label className="block text-sm font-medium text-zinc-400 mb-2">Allowed Node Types</label>
                      <div className="flex gap-2 flex-wrap">
                        {workspace.allowed_node_types.map((type) => (
                          <span key={type} className="px-3 py-1 bg-primary-500/20 text-primary-300 rounded-full text-sm">
                            {type}
                          </span>
                        ))}
                      </div>
                    </div>
                  )}
                  {workspace.created_at && (
                    <div>
                      <label className="block text-sm font-medium text-zinc-400 mb-1">Created</label>
                      <p className="text-white">{new Date(workspace.created_at).toLocaleString()}</p>
                    </div>
                  )}
                </div>
              )}
            </div>
          )}

          {/* Content Tab */}
          {activeTab === 'content' && (
            <div>
              <div className="flex justify-between items-center mb-4">
                <h3 className="text-lg font-semibold text-white">Content Tree</h3>
                <div className="flex items-center gap-3">
                  <Link
                    to={`/${repo}/content/${branch}/${workspace.name}`}
                    className="flex items-center gap-2 px-4 py-2 bg-secondary-500 hover:bg-secondary-600 text-white rounded-lg transition-colors"
                  >
                    <FolderTree className="w-4 h-4" />
                    Open Explorer
                  </Link>
                  <button
                  onClick={() => {
                    setCreateNodeParent(null)
                    setShowCreateNode(true)
                  }}
                  className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                >
                    <Plus className="w-4 h-4" />
                    Create Root Node
                  </button>
                </div>
              </div>
              <TreeView
                nodes={nodes}
                expandedNodes={expandedNodes}
                onNodeClick={(node) => setSelectedNode(node)}
                onNodeExpand={(node) => handleNodeExpand(node)}
                onEdit={(node) => {
                  setSelectedNode(node)
                  setShowNodeEditor(true)
                }}
                onAddChild={(node) => {
                  setCreateNodeParent(node)
                  setShowCreateNode(true)
                }}
                onCopy={(node) => {
                  setNodeToCopy(node)
                  setShowCopyModal(true)
                }}
                onMove={(node) => {
                  setNodeToMove(node)
                  setShowMoveModal(true)
                }}
                onPublish={handlePublish}
                onUnpublish={handleUnpublish}
                onDelete={handleDeleteNode}
                selectedNodeId={selectedNode?.id}
              />
              {selectedNode && !showNodeEditor && (
                <div className="mt-4 p-4 bg-white/5 rounded-lg">
                  <div className="flex justify-between items-start mb-2">
                    <div>
                      <h4 className="text-white font-semibold">{selectedNode.name}</h4>
                      <p className="text-sm text-zinc-400">{selectedNode.path}</p>
                    </div>
                    <div className="flex gap-2">
                      <button
                        onClick={() => {
                          setCreateNodeParent(selectedNode)
                          setShowCreateNode(true)
                        }}
                        className="p-2 bg-green-500/20 hover:bg-green-500/30 text-green-400 rounded-lg transition-colors"
                        title="Create child node"
                      >
                        <Plus className="w-4 h-4" />
                      </button>
                      <button
                        onClick={() => setShowNodeEditor(true)}
                        className="p-2 bg-primary-500/20 hover:bg-primary-500/30 text-primary-400 rounded-lg transition-colors"
                        title="Edit node"
                      >
                        <FileEdit className="w-4 h-4" />
                      </button>
                      <button
                        onClick={() => handleDeleteNode(selectedNode)}
                        className="p-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors"
                        title="Delete node"
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                  <p className="text-sm text-zinc-400">Type: <span className="text-primary-300">{selectedNode.node_type}</span></p>
                </div>
              )}
            </div>
          )}

          {/* Config Tab */}
          {activeTab === 'config' && (
            <div>
              <h3 className="text-lg font-semibold text-white mb-4">Workspace Configuration</h3>
              <WorkspaceConfigEditor workspaceName={workspaceName} repoId={repo} />
            </div>
          )}

          {/* Settings Tab */}
          {activeTab === 'settings' && (
            <div>
              <h3 className="text-lg font-semibold text-white mb-4">Advanced Settings (YAML)</h3>
              <YamlEditor
                value={yamlContent}
                onChange={(value) => setYamlContent(value || '')}
                height="500px"
              />
              <div className="mt-4">
                <button
                  onClick={handleSaveWorkspace}
                  className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                >
                  <Save className="w-4 h-4" />
                  Save YAML
                </button>
              </div>
            </div>
          )}
        </Tabs>
      </GlassCard>

      {/* Create Node Dialog */}
      {showCreateNode && (
        <CreateNodeDialog
          repo={repo!}
          branch={branch!}
          workspace={workspaceName!}
          parentPath={createNodeParent?.path}
          parentName={createNodeParent?.name}
          allowedChildren={undefined} // Will default to all available types
          onClose={() => {
            setShowCreateNode(false)
            setCreateNodeParent(null)
          }}
          onCreate={handleCreateNode}
        />
      )}

      {/* Node Editor */}
      {showNodeEditor && (
        <NodeEditor
          node={selectedNode}
          onSave={handleNodeUpdate}
          onClose={() => setShowNodeEditor(false)}
        />
      )}

      {/* Delete Confirmation */}
      {showDeleteConfirm && (
        <ConfirmDialog
          open={true}
          title="Delete Workspace"
          message={`Are you sure you want to delete workspace "${workspace.name}"? This will delete all content and cannot be undone.`}
          confirmText="Delete"
          variant="danger"
          onConfirm={handleDeleteWorkspace}
          onCancel={() => setShowDeleteConfirm(false)}
        />
      )}

      {/* Copy Node Modal */}
      {showCopyModal && nodeToCopy && (
        <CopyNodeModal
          node={nodeToCopy}
          allNodes={nodes}
          onCopy={handleCopy}
          onClose={() => {
            setShowCopyModal(false)
            setNodeToCopy(null)
          }}
        />
      )}

      {/* Move Node Modal */}
      {showMoveModal && nodeToMove && (
        <MoveNodeModal
          node={nodeToMove}
          allNodes={nodes}
          onMove={handleMove}
          onClose={() => {
            setShowMoveModal(false)
            setNodeToMove(null)
          }}
        />
      )}

      {/* Delete Node Confirmation */}
      <ConfirmDialog
        open={deleteNodeConfirm !== null}
        title="Confirm Deletion"
        message={deleteNodeConfirm?.message || ''}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          deleteNodeConfirm?.onConfirm()
          setDeleteNodeConfirm(null)
        }}
        onCancel={() => setDeleteNodeConfirm(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
