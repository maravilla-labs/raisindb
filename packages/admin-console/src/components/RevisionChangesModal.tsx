import { useState, useEffect } from 'react'
import { X, Plus, Edit, Trash2, AlertCircle, Loader2 } from 'lucide-react'
import { NodeChange } from '../api/revisions'
import { api } from '../api/client'

interface RevisionChangesModalProps {
  repo: string
  branch: string
  workspace: string
  revision: string  // HLC format: "timestamp-counter"
  changes: NodeChange[]
  onClose: () => void
}

interface NodeDetails {
  id: string
  type: string
  [key: string]: unknown
}

export default function RevisionChangesModal({
  repo,
  branch,
  workspace,
  revision,
  changes,
  onClose,
}: RevisionChangesModalProps) {
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [nodeDetails, setNodeDetails] = useState<NodeDetails | null>(null)
  const [loadingDetails, setLoadingDetails] = useState(false)
  const [detailsError, setDetailsError] = useState<string | null>(null)

  // Load node details when a node is selected
  useEffect(() => {
    if (!selectedNodeId) {
      setNodeDetails(null)
      return
    }

    async function loadNodeDetails() {
      setLoadingDetails(true)
      setDetailsError(null)
      try {
        // Try to load from current revision first
        const details = await api.get<NodeDetails>(
          `/api/repository/${repo}/${branch}/rev/${revision}/${workspace}/$ref/${selectedNodeId}`
        )
        setNodeDetails(details)
      } catch (error) {
        console.error('Failed to load node details:', error)
        setDetailsError('Failed to load node details')
      } finally {
        setLoadingDetails(false)
      }
    }

    loadNodeDetails()
  }, [selectedNodeId, repo, branch, workspace, revision])

  const getOperationIcon = (operation: string) => {
    switch (operation) {
      case 'added':
        return <Plus className="w-4 h-4" />
      case 'modified':
        return <Edit className="w-4 h-4" />
      case 'deleted':
        return <Trash2 className="w-4 h-4" />
      default:
        return null
    }
  }

  const getOperationColor = (operation: string) => {
    switch (operation) {
      case 'added':
        return 'text-green-600 bg-green-50'
      case 'modified':
        return 'text-yellow-600 bg-yellow-50'
      case 'deleted':
        return 'text-red-600 bg-red-50'
      default:
        return 'text-gray-600 bg-gray-50'
    }
  }

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg shadow-xl max-w-4xl w-full max-h-[90vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b">
          <div>
            <h2 className="text-xl font-semibold text-gray-900">
              Changes in Revision #{revision}
            </h2>
            <p className="text-sm text-gray-500 mt-1">
              {changes.length} {changes.length === 1 ? 'change' : 'changes'}
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-gray-600 transition-colors"
          >
            <X className="w-6 h-6" />
          </button>
        </div>

        {/* Content */}
        <div className="flex flex-1 overflow-hidden">
          {/* Changes List */}
          <div className="w-1/2 border-r overflow-y-auto">
            {changes.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full text-gray-500">
                <AlertCircle className="w-12 h-12 mb-2" />
                <p>No changes in this revision</p>
              </div>
            ) : (
              <div className="divide-y">
                {changes.map((change) => (
                  <button
                    key={change.node_id}
                    onClick={() => setSelectedNodeId(change.node_id)}
                    className={`w-full text-left p-4 hover:bg-gray-50 transition-colors ${
                      selectedNodeId === change.node_id ? 'bg-blue-50' : ''
                    }`}
                  >
                    <div className="flex items-start space-x-3">
                      <div
                        className={`p-2 rounded ${getOperationColor(
                          change.operation
                        )}`}
                      >
                        {getOperationIcon(change.operation)}
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center space-x-2">
                          <span className="font-medium text-gray-900 capitalize">
                            {change.operation}
                          </span>
                          {change.node_type && (
                            <span className="text-xs text-gray-500 bg-gray-100 px-2 py-0.5 rounded">
                              {change.node_type}
                            </span>
                          )}
                        </div>
                        {change.path && (
                          <p className="text-sm text-gray-600 mt-1 truncate">
                            {change.path}
                          </p>
                        )}
                        <p className="text-xs text-gray-400 mt-1 font-mono truncate">
                          {change.node_id}
                        </p>
                      </div>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Node Details */}
          <div className="w-1/2 overflow-y-auto bg-gray-50">
            {!selectedNodeId ? (
              <div className="flex items-center justify-center h-full text-gray-500">
                <p>Select a change to view details</p>
              </div>
            ) : loadingDetails ? (
              <div className="flex items-center justify-center h-full">
                <Loader2 className="w-8 h-8 animate-spin text-blue-500" />
              </div>
            ) : detailsError ? (
              <div className="flex flex-col items-center justify-center h-full text-red-500">
                <AlertCircle className="w-12 h-12 mb-2" />
                <p>{detailsError}</p>
              </div>
            ) : nodeDetails ? (
              <div className="p-6">
                <h3 className="text-lg font-semibold text-gray-900 mb-4">
                  Node Details
                </h3>
                <div className="space-y-4">
                  <div>
                    <label className="text-xs text-gray-500 uppercase">ID</label>
                    <p className="font-mono text-sm text-gray-900 break-all">
                      {nodeDetails.id}
                    </p>
                  </div>
                  <div>
                    <label className="text-xs text-gray-500 uppercase">Type</label>
                    <p className="text-sm text-gray-900">{nodeDetails.type}</p>
                  </div>
                  {Object.entries(nodeDetails)
                    .filter(([key]) => key !== 'id' && key !== 'type')
                    .map(([key, value]) => (
                      <div key={key}>
                        <label className="text-xs text-gray-500 uppercase">
                          {key}
                        </label>
                        <p className="text-sm text-gray-900">
                          {typeof value === 'object'
                            ? JSON.stringify(value, null, 2)
                            : String(value)}
                        </p>
                      </div>
                    ))}
                </div>
              </div>
            ) : null}
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end p-4 border-t bg-gray-50">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  )
}
