import { useState, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { nodesApi, type NodeRelationships, type AddRelationRequest } from '../api/nodes'
import { Plus, X, Loader2, Link2, ArrowRight, Pencil, Check } from 'lucide-react'
import { NodeBrowser } from './NodeBrowser'
import ConfirmDialog from './ConfirmDialog'

interface RelationshipManagerProps {
  repo: string
  branch: string
  workspace: string
  nodePath: string
  onClose?: () => void
}

interface ResolvedNode {
  id: string
  name: string
  path: string
  node_type: string
}

export function RelationshipManager({ repo, branch, workspace, nodePath, onClose }: RelationshipManagerProps) {
  const navigate = useNavigate()
  const [relationships, setRelationships] = useState<NodeRelationships | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [showNodeBrowser, setShowNodeBrowser] = useState(false)

  // Add relation form state
  const [targetWorkspace, setTargetWorkspace] = useState(workspace)
  const [targetPath, setTargetPath] = useState('')
  const [targetNodeName, setTargetNodeName] = useState('')
  const [_targetNodeType, setTargetNodeType] = useState('') // Stored for relation type generation
  const [weight, setWeight] = useState<string>('')
  const [relationType, setRelationType] = useState('')
  const [isEditingRelationType, setIsEditingRelationType] = useState(false)

  // Resolved node data (for displaying names/paths instead of IDs)
  const [resolvedOutgoing, setResolvedOutgoing] = useState<Map<string, ResolvedNode>>(new Map())
  const [resolvedIncoming, setResolvedIncoming] = useState<Map<string, ResolvedNode>>(new Map())
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)

  const loadRelationships = async () => {
    try {
      setLoading(true)
      setError(null)
      const data = await nodesApi.getRelationships(repo, branch, workspace, nodePath)
      setRelationships(data)

      // Resolve outgoing relationship targets
      const outgoingMap = new Map<string, ResolvedNode>()
      for (const rel of data.outgoing) {
        try {
          const node = await nodesApi.getByIdAtHead(repo, branch, rel.workspace, rel.target)
          if (node) {
            outgoingMap.set(rel.target, {
              id: node.id,
              name: node.name,
              path: node.path,
              node_type: node.node_type
            })
          }
        } catch (e) {
          console.error(`Failed to resolve outgoing node ${rel.target}:`, e)
        }
      }
      setResolvedOutgoing(outgoingMap)

      // Resolve incoming relationship sources
      const incomingMap = new Map<string, ResolvedNode>()
      for (const rel of data.incoming) {
        try {
          const node = await nodesApi.getByIdAtHead(repo, branch, rel.source_workspace, rel.source_node_id)
          if (node) {
            incomingMap.set(rel.source_node_id, {
              id: node.id,
              name: node.name,
              path: node.path,
              node_type: node.node_type
            })
          }
        } catch (e) {
          console.error(`Failed to resolve incoming node ${rel.source_node_id}:`, e)
        }
      }
      setResolvedIncoming(incomingMap)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load relationships')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadRelationships()
  }, [repo, branch, workspace, nodePath])

  const handleNodeSelect = (selectedWorkspace: string, selectedPath: string, nodeName: string, nodeType: string) => {
    setTargetWorkspace(selectedWorkspace)
    setTargetPath(selectedPath)
    setTargetNodeName(nodeName)
    setTargetNodeType(nodeType)
    // Use semantic relationship type "references" as default
    setRelationType('references')
    setShowNodeBrowser(false)
  }

  const handleAddRelation = async () => {
    if (!targetPath.trim()) {
      setError('Please select a target node')
      return
    }

    try {
      setError(null)
      const request: AddRelationRequest = {
        targetWorkspace: targetWorkspace,
        targetPath: targetPath,
        weight: weight ? parseFloat(weight) : undefined,
        relationType: relationType || undefined, // Send custom relation type if set
      }

      await nodesApi.addRelation(repo, branch, workspace, nodePath, request)

      // Reset form
      setTargetPath('')
      setTargetNodeName('')
      setTargetNodeType('')
      setWeight('')
      setRelationType('')
      setIsEditingRelationType(false)

      // Reload relationships
      await loadRelationships()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to add relationship')
    }
  }

  const handleClearSelection = () => {
    setTargetPath('')
    setTargetNodeName('')
    setTargetNodeType('')
    setTargetWorkspace(workspace)
    setWeight('')
    setRelationType('')
    setIsEditingRelationType(false)
  }

  const handleNavigateToNode = (targetWorkspace: string, targetPath: string) => {
    // Navigate to the target node in the content explorer
    navigate(`/${repo}/content/${branch}/${targetWorkspace}${targetPath}`)
  }

  const handleRemoveRelation = async (targetWs: string, targetNodeId: string) => {
    const resolved = resolvedOutgoing.get(targetNodeId)
    const displayName = resolved ? `${targetWs}:${resolved.path}` : `${targetWs}:${targetNodeId}`

    setDeleteConfirm({
      message: `Remove relationship to ${displayName}?`,
      onConfirm: async () => {
        try {
          setError(null)
          // Note: The API expects targetPath, but we're using the resolved path
          const targetPath = resolved?.path || targetNodeId
          await nodesApi.removeRelation(repo, branch, workspace, nodePath, {
            targetWorkspace: targetWs,
            targetPath: targetPath,
          })

          // Reload relationships
          await loadRelationships()
        } catch (err) {
          setError(err instanceof Error ? err.message : 'Failed to remove relationship')
        }
      }
    })
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <Loader2 className="w-6 h-6 animate-spin text-blue-500" />
        <span className="ml-2 text-gray-400">Loading relationships...</span>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-white">Relationships</h3>
        {onClose && (
          <button
            onClick={onClose}
            className="p-1 hover:bg-white/10 rounded transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        )}
      </div>

      {/* Error Display */}
      {error && (
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4">
          <p className="text-red-400 text-sm">{error}</p>
        </div>
      )}

      {/* Add Relationship Section */}
      <div className="bg-white/5 rounded-lg p-4 space-y-4 border border-white/10">
        <h4 className="text-sm font-semibold text-white">Add New Relationship</h4>

        {/* Selected Node Display */}
        {targetPath ? (
          <div className="bg-primary-500/20 border border-primary-400/30 rounded-lg p-3">
            <div className="flex items-start justify-between gap-2">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <Link2 className="w-4 h-4 text-primary-400 flex-shrink-0" />
                  <span className="text-sm font-medium text-white truncate">
                    {targetNodeName}
                  </span>
                </div>
                <div className="text-xs text-primary-300 font-mono truncate">
                  {targetWorkspace}:{targetPath}
                </div>
              </div>
              <button
                onClick={handleClearSelection}
                className="p-1 hover:bg-white/10 rounded transition-colors flex-shrink-0"
                aria-label="Clear selection"
              >
                <X className="w-4 h-4 text-primary-400" />
              </button>
            </div>
          </div>
        ) : (
          <button
            onClick={() => setShowNodeBrowser(true)}
            className="flex items-center gap-2 px-4 py-3 bg-primary-500/20 hover:bg-primary-500/30
                       border border-primary-400/30 rounded-lg transition-all w-full justify-center
                       text-primary-300 hover:text-primary-200
                       focus:outline-none focus:ring-2 focus:ring-primary-500/50"
            aria-label="Browse and select target node"
          >
            <Plus className="w-4 h-4" />
            <span className="font-medium">Select Target Node</span>
          </button>
        )}

        {/* Relation Type (only shown when node is selected) */}
        {targetPath && (
          <div>
            <label className="block text-sm font-medium text-zinc-300 mb-2">
              Relation Type
            </label>
            <div className="flex items-center gap-2">
              {!isEditingRelationType ? (
                <>
                  <div className="flex-1 px-3 py-2 bg-purple-500/20 border border-purple-400/30 rounded-lg text-purple-300 font-mono text-sm">
                    {relationType}
                  </div>
                  <button
                    onClick={() => setIsEditingRelationType(true)}
                    className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                    title="Edit relation type"
                    aria-label="Edit relation type"
                  >
                    <Pencil className="w-4 h-4 text-purple-400" />
                  </button>
                </>
              ) : (
                <>
                  <input
                    type="text"
                    value={relationType}
                    onChange={(e) => setRelationType(e.target.value)}
                    className="flex-1 px-3 py-2 bg-black/30 border border-purple-400/50 rounded-lg
                             text-white font-mono text-sm
                             focus:outline-none focus:ring-2 focus:ring-purple-500/50 focus:border-purple-500/50
                             transition-all"
                    placeholder="ntRaisinFolder"
                    aria-label="Custom relation type"
                    autoFocus
                  />
                  <button
                    onClick={() => setIsEditingRelationType(false)}
                    className="p-2 bg-purple-500 hover:bg-purple-600 rounded-lg transition-colors"
                    title="Confirm"
                    aria-label="Confirm relation type"
                  >
                    <Check className="w-4 h-4 text-white" />
                  </button>
                </>
              )}
            </div>
            <p className="text-xs text-zinc-500 mt-1">
              Auto-generated from node type. Cypher-compatible format (e.g., ntRaisinFolder)
            </p>
          </div>
        )}

        {/* Weight Input (only shown when node is selected) */}
        {targetPath && (
          <div>
            <label className="block text-sm font-medium text-zinc-300 mb-2">
              Weight (optional)
            </label>
            <input
              type="number"
              step="0.1"
              value={weight}
              onChange={(e) => setWeight(e.target.value)}
              className="w-full px-3 py-2 bg-black/30 border border-white/10 rounded-lg
                       text-white placeholder-zinc-500
                       focus:outline-none focus:ring-2 focus:ring-primary-500/50 focus:border-primary-500/50
                       transition-all"
              placeholder="1.0"
              aria-label="Relationship weight"
            />
            <p className="text-xs text-zinc-500 mt-1">
              Optional numeric weight for graph algorithms
            </p>
          </div>
        )}

        {/* Add Button (only shown when node is selected) */}
        {targetPath && (
          <button
            onClick={handleAddRelation}
            className="w-full px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg
                     transition-all font-medium
                     focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2
                     focus:ring-offset-zinc-900
                     active:scale-95"
          >
            Add Relationship
          </button>
        )}
      </div>

      {/* Node Browser Modal */}
      {showNodeBrowser && (
        <NodeBrowser
          repo={repo}
          branch={branch}
          currentWorkspace={workspace}
          onSelect={handleNodeSelect}
          onClose={() => setShowNodeBrowser(false)}
          excludePath={nodePath}
        />
      )}

      {/* Relationships Display */}
      {relationships && (
        <div className="space-y-4">
          {/* Outgoing Relationships */}
          <div>
            <h4 className="text-sm font-semibold text-gray-400 mb-3">
              Outgoing ({relationships.outgoing.length})
            </h4>
            {relationships.outgoing.length === 0 ? (
              <p className="text-gray-500 text-sm italic">No outgoing relationships</p>
            ) : (
              <div className="space-y-2">
                {relationships.outgoing.map((rel, idx) => {
                  const resolved = resolvedOutgoing.get(rel.target)
                  return (
                    <div
                      key={idx}
                      className="flex items-center justify-between p-3 bg-white/5 rounded-lg border border-white/10
                                hover:border-primary-500/30 transition-all group"
                    >
                      <button
                        onClick={() => resolved && handleNavigateToNode(rel.workspace, resolved.path)}
                        disabled={!resolved}
                        className="flex-1 text-left cursor-pointer disabled:cursor-default"
                      >
                        <div className="flex items-center gap-2">
                          <ArrowRight className="w-4 h-4 text-blue-400 group-hover:text-blue-300 transition-colors" />
                          <div className="flex-1 min-w-0">
                            {resolved ? (
                              <>
                                <div className="text-white text-sm font-medium truncate">
                                  {resolved.name}
                                </div>
                                <div className="text-xs text-zinc-400 font-mono truncate">
                                  {rel.workspace}:{resolved.path}
                                </div>
                              </>
                            ) : (
                              <div className="text-zinc-400 font-mono text-sm">
                                {rel.workspace}:{rel.target}
                              </div>
                            )}
                          </div>
                        </div>
                        <div className="flex items-center gap-3 mt-2">
                          <span className="text-xs px-2 py-0.5 bg-purple-500/20 text-purple-300 rounded">
                            {resolved?.node_type || rel.relation_type}
                          </span>
                          {rel.weight !== undefined && (
                            <span className="text-xs text-gray-400">
                              weight: {rel.weight}
                            </span>
                          )}
                        </div>
                      </button>
                      <button
                        onClick={() => handleRemoveRelation(rel.workspace, rel.target)}
                        className="p-2 hover:bg-red-500/20 rounded transition-colors ml-2 flex-shrink-0"
                        title="Remove relationship"
                      >
                        <X className="w-4 h-4 text-red-400" />
                      </button>
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          {/* Incoming Relationships */}
          <div>
            <h4 className="text-sm font-semibold text-gray-400 mb-3">
              Incoming ({relationships.incoming.length})
            </h4>
            {relationships.incoming.length === 0 ? (
              <p className="text-gray-500 text-sm italic">No incoming relationships</p>
            ) : (
              <div className="space-y-2">
                {relationships.incoming.map((rel, idx) => {
                  const resolved = resolvedIncoming.get(rel.source_node_id)
                  return (
                    <div
                      key={idx}
                      className="flex items-center justify-between p-3 bg-white/5 rounded-lg border border-white/10
                                hover:border-green-500/30 transition-all group"
                    >
                      <button
                        onClick={() => resolved && handleNavigateToNode(rel.source_workspace, resolved.path)}
                        disabled={!resolved}
                        className="flex-1 text-left cursor-pointer disabled:cursor-default"
                      >
                        <div className="flex items-center gap-2">
                          <ArrowRight className="w-4 h-4 text-green-400 group-hover:text-green-300 transition-colors rotate-180" />
                          <div className="flex-1 min-w-0">
                            {resolved ? (
                              <>
                                <div className="text-white text-sm font-medium truncate">
                                  {resolved.name}
                                </div>
                                <div className="text-xs text-zinc-400 font-mono truncate">
                                  {rel.source_workspace}:{resolved.path}
                                </div>
                              </>
                            ) : (
                              <div className="text-zinc-400 font-mono text-sm">
                                {rel.source_workspace}:{rel.source_node_id}
                              </div>
                            )}
                          </div>
                        </div>
                        <div className="flex items-center gap-3 mt-2">
                          <span className="text-xs px-2 py-0.5 bg-purple-500/20 text-purple-300 rounded">
                            {resolved?.node_type || rel.relation_type}
                          </span>
                          {rel.weight !== undefined && (
                            <span className="text-xs text-gray-400">
                              weight: {rel.weight}
                            </span>
                          )}
                        </div>
                      </button>
                      <span className="text-xs px-2 py-1 bg-green-500/10 text-green-400 rounded flex-shrink-0 ml-2">
                        Referenced by
                      </span>
                    </div>
                  )
                })}
              </div>
            )}
          </div>
        </div>
      )}
      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Confirm Removal"
        message={deleteConfirm?.message || ''}
        variant="warning"
        confirmText="Remove"
        onConfirm={() => {
          deleteConfirm?.onConfirm()
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />
    </div>
  )
}
