import { useState, useEffect } from 'react'
import { Clock, MessageSquare, RotateCcw, Trash2, Edit2, Plus, X, Check } from 'lucide-react'
import ConfirmDialog from './ConfirmDialog'

interface NodeVersion {
  id: string
  node_id: string
  version: number
  note?: string | null
  created_at: string
  updated_at?: string | null
}

interface VersionHistoryProps {
  repo: string
  branch: string
  workspace: string
  nodePath: string
  onRestore?: (node: any) => void
}

export default function VersionHistory({ repo, branch, workspace, nodePath, onRestore }: VersionHistoryProps) {
  const [versions, setVersions] = useState<NodeVersion[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [editingVersion, setEditingVersion] = useState<number | null>(null)
  const [editNote, setEditNote] = useState('')
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [createNote, setCreateNote] = useState('')
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null)
  const [confirmRestore, setConfirmRestore] = useState<number | null>(null)
  const [isCreating, setIsCreating] = useState(false)

  useEffect(() => {
    fetchVersions()
  }, [repo, branch, workspace, nodePath])

  async function fetchVersions() {
    try {
      setLoading(true)
      setError(null)
      const response = await fetch(`/api/repository/${repo}/${branch}/${workspace}${nodePath}/raisin:version`)
      if (!response.ok) throw new Error('Failed to fetch versions')
      const data = await response.json()
      // Sort versions in descending order (newest first)
      const sortedVersions = data.sort((a: NodeVersion, b: NodeVersion) => b.version - a.version)
      setVersions(sortedVersions)
    } catch (err: any) {
      setError(err.message)
    } finally {
      setLoading(false)
    }
  }

  async function handleCreateVersion() {
    try {
      setIsCreating(true)
      const response = await fetch(`/api/repository/${repo}/${branch}/${workspace}${nodePath}/raisin:cmd/create_version`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ note: createNote || null }),
      })
      if (!response.ok) throw new Error('Failed to create version')

      setShowCreateModal(false)
      setCreateNote('')
      await fetchVersions()
    } catch (err: any) {
      setError(err.message)
    } finally {
      setIsCreating(false)
    }
  }

  async function handleRestoreVersion(version: number) {
    try {
      const response = await fetch(`/api/repository/${repo}/${branch}/${workspace}${nodePath}/raisin:cmd/restore_version`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ version }),
      })
      if (!response.ok) throw new Error('Failed to restore version')

      const restoredNode = await response.json()
      setConfirmRestore(null)
      await fetchVersions()

      if (onRestore) {
        onRestore(restoredNode)
      }
    } catch (err: any) {
      setError(err.message)
    }
  }

  async function handleDeleteVersion(version: number) {
    try {
      const response = await fetch(`/api/repository/${repo}/${branch}/${workspace}${nodePath}/raisin:cmd/delete_version`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ version }),
      })
      if (!response.ok) {
        const data = await response.json()
        throw new Error(data.message || 'Failed to delete version')
      }

      setConfirmDelete(null)
      await fetchVersions()
    } catch (err: any) {
      setError(err.message)
    }
  }

  async function handleUpdateNote(version: number) {
    try {
      const response = await fetch(`/api/repository/${repo}/${branch}/${workspace}${nodePath}/raisin:cmd/update_version_note`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ version, note: editNote || null }),
      })
      if (!response.ok) throw new Error('Failed to update note')

      setEditingVersion(null)
      setEditNote('')
      await fetchVersions()
    } catch (err: any) {
      setError(err.message)
    }
  }

  function startEditNote(version: NodeVersion) {
    setEditingVersion(version.version)
    setEditNote(version.note || '')
  }

  function cancelEdit() {
    setEditingVersion(null)
    setEditNote('')
  }

  function formatDate(dateStr: string) {
    const date = new Date(dateStr)
    return date.toLocaleString()
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center p-8">
        <div className="text-gray-400">Loading versions...</div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-4 bg-red-500/20 border border-red-500/50 rounded-lg text-red-300">
        {error}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex justify-between items-center">
        <h3 className="text-lg font-semibold text-white flex items-center gap-2">
          <Clock className="w-5 h-5" />
          Version History ({versions.length})
        </h3>
        <button
          onClick={() => setShowCreateModal(true)}
          className="flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-4 h-4" />
          Create Version
        </button>
      </div>

      {versions.length === 0 ? (
        <div className="text-center text-gray-400 py-8">
          No versions yet. Create the first snapshot!
        </div>
      ) : (
        <div className="space-y-3">
          {versions.map((version, index) => (
            <div
              key={version.id}
              className="glass-dark rounded-lg p-4 hover:bg-white/5 transition-colors"
            >
              <div className="flex items-start justify-between">
                <div className="flex-1">
                  <div className="flex items-center gap-3">
                    <span className="text-xl font-bold text-purple-400">
                      v{version.version}
                    </span>
                    {index === 0 && (
                      <span className="px-2 py-1 bg-green-500/20 text-green-300 text-xs rounded">
                        Latest
                      </span>
                    )}
                  </div>

                  <div className="mt-2 text-sm text-gray-400">
                    {formatDate(version.created_at)}
                  </div>

                  {editingVersion === version.version ? (
                    <div className="mt-3 flex items-center gap-2">
                      <input
                        type="text"
                        value={editNote}
                        onChange={(e) => setEditNote(e.target.value)}
                        placeholder="Add a note..."
                        className="flex-1 px-3 py-2 bg-white/10 border border-white/20 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-purple-500"
                        autoFocus
                      />
                      <button
                        onClick={() => handleUpdateNote(version.version)}
                        className="p-2 bg-green-500 hover:bg-green-600 text-white rounded-lg"
                      >
                        <Check className="w-4 h-4" />
                      </button>
                      <button
                        onClick={cancelEdit}
                        className="p-2 bg-gray-500 hover:bg-gray-600 text-white rounded-lg"
                      >
                        <X className="w-4 h-4" />
                      </button>
                    </div>
                  ) : version.note ? (
                    <div className="mt-3 flex items-start gap-2 p-3 bg-white/5 rounded-lg group">
                      <MessageSquare className="w-4 h-4 text-gray-400 mt-0.5" />
                      <p className="flex-1 text-gray-300">{version.note}</p>
                      <button
                        onClick={() => startEditNote(version)}
                        className="opacity-0 group-hover:opacity-100 p-1 hover:bg-white/10 rounded transition-all"
                      >
                        <Edit2 className="w-3 h-3 text-gray-400" />
                      </button>
                    </div>
                  ) : (
                    <button
                      onClick={() => startEditNote(version)}
                      className="mt-3 flex items-center gap-2 text-sm text-gray-500 hover:text-gray-300 transition-colors"
                    >
                      <MessageSquare className="w-4 h-4" />
                      Add note...
                    </button>
                  )}
                </div>

                <div className="flex items-center gap-2">
                  <button
                    onClick={() => setConfirmRestore(version.version)}
                    className="p-2 hover:bg-blue-500/20 text-blue-400 rounded-lg transition-colors"
                    title="Restore this version"
                  >
                    <RotateCcw className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => setConfirmDelete(version.version)}
                    className="p-2 hover:bg-red-500/20 text-red-400 rounded-lg transition-colors"
                    title="Delete this version"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Create Version Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
          <div className="glass-dark rounded-xl max-w-md w-full p-6">
            <h3 className="text-xl font-bold text-white mb-4">Create New Version</h3>
            <p className="text-gray-400 text-sm mb-4">
              This will create a snapshot of the current node state.
            </p>
            <textarea
              value={createNote}
              onChange={(e) => setCreateNote(e.target.value)}
              placeholder="Add a note describing this version (optional)"
              className="w-full px-3 py-2 bg-white/10 border border-white/20 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-purple-500 resize-none"
              rows={3}
            />
            <div className="flex gap-3 justify-end mt-6">
              <button
                onClick={() => {
                  setShowCreateModal(false)
                  setCreateNote('')
                }}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
                disabled={isCreating}
              >
                Cancel
              </button>
              <button
                onClick={handleCreateVersion}
                disabled={isCreating}
                className="px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors disabled:opacity-50"
              >
                {isCreating ? 'Creating...' : 'Create Version'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Restore Confirmation */}
      <ConfirmDialog
        open={confirmRestore !== null}
        title="Restore Version"
        message={`Are you sure you want to restore to version ${confirmRestore}? This will create a new version based on the selected snapshot.`}
        confirmText="Restore"
        onConfirm={() => confirmRestore !== null && handleRestoreVersion(confirmRestore)}
        onCancel={() => setConfirmRestore(null)}
      />

      {/* Delete Confirmation */}
      <ConfirmDialog
        open={confirmDelete !== null}
        title="Delete Version"
        message={`Are you sure you want to delete version ${confirmDelete}? This action cannot be undone.`}
        confirmText="Delete"
        variant="danger"
        onConfirm={() => confirmDelete !== null && handleDeleteVersion(confirmDelete)}
        onCancel={() => setConfirmDelete(null)}
      />
    </div>
  )
}
