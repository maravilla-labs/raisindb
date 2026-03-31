import { useState, useEffect } from 'react'
import { Save, GitBranch, Package, AlertCircle, Plus, Trash2, Pin, PinOff, Settings, Clock, Zap, Trash } from 'lucide-react'
import { workspacesApi, type WorkspaceConfig, type WorkspaceMode } from '../api/workspaces'
import { branchesApi, type Branch } from '../api/branches'
import { nodeTypesApi } from '../api/nodetypes'

interface WorkspaceConfigEditorProps {
  workspaceName: string
  repoId: string
}

interface NodeTypeOption {
  name: string
  currentRevision: number
}

export default function WorkspaceConfigEditor({ workspaceName, repoId }: WorkspaceConfigEditorProps) {
  const [config, setConfig] = useState<WorkspaceConfig | null>(null)
  const [branches, setBranches] = useState<Branch[]>([])
  const [nodeTypes, setNodeTypes] = useState<NodeTypeOption[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState(false)
  const [selectedNodeType, setSelectedNodeType] = useState('')

  useEffect(() => {
    loadData()
  }, [workspaceName, repoId])

  async function loadData() {
    setLoading(true)
    try {
      const [configData, branchesData, nodeTypesData] = await Promise.all([
        workspacesApi.getConfig(repoId, workspaceName),
        branchesApi.list(repoId),
        nodeTypesApi.list(repoId, 'main')  // Use main branch for NodeTypes
      ])
      
      // Ensure mode is set (backward compatibility)
      if (!configData.mode) {
        configData.mode = { type: 'Versioned', base_revision: 0, auto_commit: false }
      }
      
      // Ensure workspace_id is set
      if (!configData.workspace_id) {
        configData.workspace_id = workspaceName
      }
      
      // Handle legacy node_type_refs
      if (!configData.node_type_pins && configData.node_type_refs) {
        configData.node_type_pins = configData.node_type_refs
      }
      
      setConfig(configData)
      setBranches(branchesData)
      
      // Transform node types to include current revision
      const nodeTypeOptions = nodeTypesData.map(nt => ({
        name: nt.name,
        currentRevision: nt.version || 1
      }))
      setNodeTypes(nodeTypeOptions)
    } catch (error) {
      console.error('Failed to load config:', error)
      setError('Failed to load workspace configuration')
    } finally {
      setLoading(false)
    }
  }

  async function handleSave() {
    if (!config) return
    
    setSaving(true)
    setError(null)
    setSuccess(false)
    
    try {
      await workspacesApi.updateConfig(repoId, workspaceName, config)
      setSuccess(true)
      setTimeout(() => setSuccess(false), 3000)
    } catch (err: any) {
      setError(err.message || 'Failed to save configuration')
    } finally {
      setSaving(false)
    }
  }

  function handleDefaultBranchChange(branchName: string) {
    if (!config) return
    setConfig({ ...config, default_branch: branchName })
  }

  function handlePinNodeType(nodeTypeName: string, revision: number | null) {
    if (!config) return
    setConfig({
      ...config,
      node_type_pins: {
        ...config.node_type_pins,
        [nodeTypeName]: revision
      }
    })
  }

  function handleUnpinNodeType(nodeTypeName: string) {
    if (!config) return
    const newRefs = { ...config.node_type_pins }
    delete newRefs[nodeTypeName]
    setConfig({ ...config, node_type_pins: newRefs })
  }

  function handleModeChange(mode: WorkspaceMode) {
    if (!config) return
    setConfig({ ...config, mode })
  }

  function handleAddNodeTypePin() {
    if (!selectedNodeType || !config) return
    
    const nodeType = nodeTypes.find(nt => nt.name === selectedNodeType)
    if (!nodeType) return
    
    // Pin to current revision by default
    handlePinNodeType(selectedNodeType, nodeType.currentRevision)
    setSelectedNodeType('')
  }

  if (loading) {
    return (
      <div className="text-center text-gray-400 py-12">
        <div className="w-8 h-8 border-2 border-primary-400 border-t-transparent rounded-full animate-spin mx-auto mb-3" />
        Loading configuration...
      </div>
    )
  }

  if (!config) {
    return (
      <div className="text-center text-gray-400 py-12">
        <AlertCircle className="w-12 h-12 text-gray-600 mx-auto mb-3" />
        <p>No configuration found</p>
      </div>
    )
  }

  const nodeTypePins = config.node_type_pins || config.node_type_refs || {}
  const pinnedNodeTypes = Object.entries(nodeTypePins)
  const unpinnedNodeTypes = nodeTypes.filter(nt => !(nt.name in nodeTypePins))

  return (
    <div className="space-y-6">
      {/* Success Message */}
      {success && (
        <div className="flex items-center gap-2 p-3 bg-green-500/10 border border-green-500/20 rounded-lg">
          <AlertCircle className="w-4 h-4 text-green-400 flex-shrink-0" />
          <p className="text-sm text-green-400">Configuration saved successfully!</p>
        </div>
      )}

      {/* Error Message */}
      {error && (
        <div className="flex items-center gap-2 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
          <AlertCircle className="w-4 h-4 text-red-400 flex-shrink-0" />
          <p className="text-sm text-red-400">{error}</p>
        </div>
      )}

      {/* Workspace Mode */}
      <div>
        <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-3">
          <Settings className="w-4 h-4 text-purple-400" />
          Workspace Mode
        </label>
        <p className="text-xs text-gray-500 mb-4">
          Controls how content changes are tracked and committed in this workspace
        </p>

        <div className="space-y-3">
          {/* Versioned Mode */}
          <div
            className={`p-4 border rounded-lg cursor-pointer transition-all ${
              config.mode.type === 'Versioned'
                ? 'bg-purple-500/10 border-purple-500/50'
                : 'bg-white/5 border-white/10 hover:border-white/20'
            }`}
            onClick={() => handleModeChange({ type: 'Versioned', base_revision: 0, auto_commit: false })}
          >
            <div className="flex items-start gap-3">
              <Clock className="w-5 h-5 text-purple-400 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-medium text-white">Versioned</span>
                  <span className="text-xs px-2 py-0.5 bg-purple-500/20 text-purple-300 rounded">Recommended</span>
                </div>
                <p className="text-xs text-gray-400 mb-3">
                  Full revision tracking with immutable trees. Use for editorial content, versioned APIs, and audited workflows.
                </p>
                
                {config.mode.type === 'Versioned' && (
                  <div className="space-y-3 pt-3 border-t border-white/10">
                    <div>
                      <label className="flex items-center gap-2 text-xs text-gray-400 mb-2">
                        Base Revision
                      </label>
                      <input
                        type="number"
                        min="0"
                        value={config.mode.base_revision}
                        onChange={(e) => handleModeChange({ 
                          type: 'Versioned', 
                          base_revision: parseInt(e.target.value) || 0,
                          auto_commit: config.mode.type === 'Versioned' ? config.mode.auto_commit : false
                        })}
                        onClick={(e) => e.stopPropagation()}
                        className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded text-white text-sm focus:outline-none focus:ring-2 focus:ring-purple-500"
                      />
                    </div>
                    
                    <label className="flex items-center gap-2 text-xs text-gray-400 cursor-pointer"
                           onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        checked={config.mode.auto_commit}
                        onChange={(e) => handleModeChange({ 
                          type: 'Versioned',
                          base_revision: config.mode.type === 'Versioned' ? config.mode.base_revision : 0,
                          auto_commit: e.target.checked
                        })}
                        className="w-4 h-4 rounded border-white/20 bg-black/30 text-purple-500 focus:ring-2 focus:ring-purple-500"
                      />
                      <span>
                        Auto-commit on every change
                        <span className="text-gray-500 ml-1">(expensive but simple)</span>
                      </span>
                    </label>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Live Mode */}
          <div
            className={`p-4 border rounded-lg cursor-pointer transition-all ${
              config.mode.type === 'Live'
                ? 'bg-green-500/10 border-green-500/50'
                : 'bg-white/5 border-white/10 hover:border-white/20'
            }`}
            onClick={() => handleModeChange({ type: 'Live', keep_deltas: true, max_deltas: 100 })}
          >
            <div className="flex items-start gap-3">
              <Zap className="w-5 h-5 text-green-400 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-medium text-white">Live</span>
                  <span className="text-xs px-2 py-0.5 bg-green-500/20 text-green-300 rounded">Fast</span>
                </div>
                <p className="text-xs text-gray-400 mb-3">
                  Live-edit mode with delta log only (no immutable trees). Use for user profiles, sessions, real-time collaboration.
                </p>
                
                {config.mode.type === 'Live' && (
                  <div className="space-y-3 pt-3 border-t border-white/10">
                    <label className="flex items-center gap-2 text-xs text-gray-400 cursor-pointer"
                           onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        checked={config.mode.keep_deltas}
                        onChange={(e) => handleModeChange({ 
                          type: 'Live',
                          keep_deltas: e.target.checked,
                          max_deltas: config.mode.type === 'Live' ? config.mode.max_deltas : 100
                        })}
                        className="w-4 h-4 rounded border-white/20 bg-black/30 text-green-500 focus:ring-2 focus:ring-green-500"
                      />
                      <span>Keep delta history (for undo/redo)</span>
                    </label>
                    
                    {config.mode.keep_deltas && (
                      <div>
                        <label className="flex items-center gap-2 text-xs text-gray-400 mb-2">
                          Max Delta History Size
                        </label>
                        <input
                          type="number"
                          min="0"
                          value={config.mode.max_deltas}
                          onChange={(e) => handleModeChange({ 
                            type: 'Live',
                            keep_deltas: config.mode.type === 'Live' ? config.mode.keep_deltas : true,
                            max_deltas: parseInt(e.target.value) || 0
                          })}
                          onClick={(e) => e.stopPropagation()}
                          className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded text-white text-sm focus:outline-none focus:ring-2 focus:ring-green-500"
                          placeholder="0 = no history"
                        />
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Ephemeral Mode */}
          <div
            className={`p-4 border rounded-lg cursor-pointer transition-all ${
              config.mode.type === 'Ephemeral'
                ? 'bg-orange-500/10 border-orange-500/50'
                : 'bg-white/5 border-white/10 hover:border-white/20'
            }`}
            onClick={() => handleModeChange({ type: 'Ephemeral' })}
          >
            <div className="flex items-start gap-3">
              <Trash className="w-5 h-5 text-orange-400 flex-shrink-0 mt-0.5" />
              <div className="flex-1">
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-medium text-white">Ephemeral</span>
                  <span className="text-xs px-2 py-0.5 bg-orange-500/20 text-orange-300 rounded">Temporary</span>
                </div>
                <p className="text-xs text-gray-400">
                  Ephemeral workspace (deleted on close). Use for temporary previews, sandbox testing, or auto-delete branches.
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Default Branch */}
      <div>
        <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-3">
          <GitBranch className="w-4 h-4 text-primary-400" />
          Default Branch
        </label>
        <p className="text-xs text-gray-500 mb-3">
          The branch that will be used by default when accessing this workspace
        </p>
        <select
          value={config.default_branch}
          onChange={(e) => handleDefaultBranchChange(e.target.value)}
          className="w-full md:w-96 px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
        >
          {branches.map((branch) => (
            <option key={branch.name} value={branch.name}>
              {branch.name} (r{branch.head})
            </option>
          ))}
        </select>
      </div>

      {/* NodeType Version Pinning */}
      <div>
        <label className="flex items-center gap-2 text-sm font-medium text-gray-300 mb-3">
          <Package className="w-4 h-4 text-amber-400" />
          NodeType Version Pinning
        </label>
        <p className="text-xs text-gray-500 mb-4">
          Pin specific NodeTypes to specific revisions. Unpinned NodeTypes will always use the latest version.
        </p>

        {/* Pinned NodeTypes */}
        {pinnedNodeTypes.length > 0 && (
          <div className="space-y-2 mb-4">
            {pinnedNodeTypes.map(([name, revision]) => {
              const nodeType = nodeTypes.find(nt => nt.name === name)
              const isLatest = revision === null
              const currentRevision = nodeType?.currentRevision || 1
              const isPinned = revision !== null

              return (
                <div
                  key={name}
                  className="flex items-center justify-between p-3 bg-white/5 border border-white/10 rounded-lg"
                >
                  <div className="flex items-center gap-3 flex-1">
                    <Package className="w-4 h-4 text-amber-400 flex-shrink-0" />
                    <div className="flex-1">
                      <div className="text-sm font-medium text-white">{name}</div>
                      <div className="text-xs text-gray-400">
                        {isLatest ? (
                          <span className="text-green-400">Using latest (r{currentRevision})</span>
                        ) : (
                          <>
                            Pinned to <span className="text-amber-400">r{revision}</span>
                            {revision !== currentRevision && (
                              <span className="text-yellow-400 ml-2">(latest: r{currentRevision})</span>
                            )}
                          </>
                        )}
                      </div>
                    </div>
                  </div>
                  
                  <div className="flex items-center gap-2">
                    {isPinned && (
                      <select
                        value={revision || ''}
                        onChange={(e) => handlePinNodeType(name, e.target.value ? parseInt(e.target.value) : null)}
                        className="px-3 py-1 bg-black/30 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-2 focus:ring-amber-500"
                      >
                        <option value="">Latest</option>
                        {Array.from({ length: currentRevision }, (_, i) => i + 1).map(rev => (
                          <option key={rev} value={rev}>r{rev}</option>
                        ))}
                      </select>
                    )}
                    
                    {isPinned ? (
                      <button
                        onClick={() => handlePinNodeType(name, null)}
                        className="p-2 text-gray-400 hover:text-green-400 hover:bg-green-500/10 rounded-lg transition-colors"
                        title="Use latest version"
                      >
                        <PinOff className="w-4 h-4" />
                      </button>
                    ) : (
                      <button
                        onClick={() => handlePinNodeType(name, currentRevision)}
                        className="p-2 text-gray-400 hover:text-amber-400 hover:bg-amber-500/10 rounded-lg transition-colors"
                        title="Pin to specific version"
                      >
                        <Pin className="w-4 h-4" />
                      </button>
                    )}
                    
                    <button
                      onClick={() => handleUnpinNodeType(name)}
                      className="p-2 text-gray-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                      title="Remove from config"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        )}

        {/* Add NodeType Pin */}
        {unpinnedNodeTypes.length > 0 && (
          <div className="flex items-center gap-2">
            <select
              value={selectedNodeType}
              onChange={(e) => setSelectedNodeType(e.target.value)}
              className="flex-1 md:flex-none md:w-64 px-4 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-amber-500"
            >
              <option value="">Select NodeType to configure...</option>
              {unpinnedNodeTypes.map((nt) => (
                <option key={nt.name} value={nt.name}>
                  {nt.name} (r{nt.currentRevision})
                </option>
              ))}
            </select>
            <button
              onClick={handleAddNodeTypePin}
              disabled={!selectedNodeType}
              className="flex items-center gap-2 px-4 py-2 bg-amber-500 hover:bg-amber-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
            >
              <Plus className="w-4 h-4" />
              Add
            </button>
          </div>
        )}

        {pinnedNodeTypes.length === 0 && (
          <div className="text-center text-gray-500 py-8 bg-white/5 border border-white/10 rounded-lg">
            <Package className="w-8 h-8 text-gray-600 mx-auto mb-2" />
            <p className="text-sm">No NodeTypes configured yet</p>
            <p className="text-xs mt-1">All NodeTypes will use their latest versions</p>
          </div>
        )}
      </div>

      {/* Info Box */}
      <div className="flex items-start gap-2 p-3 bg-blue-500/10 border border-blue-500/20 rounded-lg">
        <AlertCircle className="w-4 h-4 text-blue-400 flex-shrink-0 mt-0.5" />
        <div className="text-xs text-blue-300">
          <p className="font-medium mb-1">About Version Pinning</p>
          <ul className="list-disc list-inside space-y-1">
            <li>Pinned NodeTypes use a specific revision, ensuring consistency</li>
            <li>Unpinned NodeTypes automatically use the latest version</li>
            <li>Use pinning to lock critical NodeTypes during production</li>
            <li>Set to "Latest" to track ongoing development</li>
          </ul>
        </div>
      </div>

      {/* Save Button */}
      <div className="flex justify-end pt-4 border-t border-white/10">
        <button
          onClick={handleSave}
          disabled={saving}
          className="flex items-center gap-2 px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-gray-600 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
        >
          {saving ? (
            <>
              <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
              Saving...
            </>
          ) : (
            <>
              <Save className="w-4 h-4" />
              Save Configuration
            </>
          )}
        </button>
      </div>
    </div>
  )
}
