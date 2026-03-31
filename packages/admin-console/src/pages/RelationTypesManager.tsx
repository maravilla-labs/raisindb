import { useEffect, useState } from 'react'
import { useParams } from 'react-router-dom'
import { Link2, Plus, Search, X } from 'lucide-react'
import { createPortal } from 'react-dom'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest } from '../api/nodes'

const WORKSPACE = 'raisin:access_control'
const BASE_PATH = '/relation-types'

interface RelationType {
  id?: string
  nodeName: string
  path: string
  relation_name: string
  title: string
  description?: string
  category: 'household' | 'organization' | 'social'
  inverse_relation_name?: string
  bidirectional: boolean
  implies_stewardship: boolean
  requires_minor: boolean
  icon?: string
  color?: string
  created_at?: string
  updated_at?: string
}

function nodeToRelationType(node: Node): RelationType {
  return {
    id: node.id,
    nodeName: node.name,
    path: node.path,
    relation_name: node.properties?.relation_name as string,
    title: node.properties?.title as string,
    description: node.properties?.description as string | undefined,
    category: (node.properties?.category as 'household' | 'organization' | 'social') || 'social',
    inverse_relation_name: node.properties?.inverse_relation_name as string | undefined,
    bidirectional: (node.properties?.bidirectional as boolean) || false,
    implies_stewardship: (node.properties?.implies_stewardship as boolean) || false,
    requires_minor: (node.properties?.requires_minor as boolean) || false,
    icon: node.properties?.icon as string | undefined,
    color: node.properties?.color as string | undefined,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

interface RelationTypeDialogProps {
  isOpen: boolean
  onClose: () => void
  onSave: (data: Omit<RelationType, 'id' | 'nodeName' | 'path' | 'created_at' | 'updated_at'>) => Promise<void>
  editingRelationType?: RelationType
}

function RelationTypeDialog({ isOpen, onClose, onSave, editingRelationType }: RelationTypeDialogProps) {
  const [formData, setFormData] = useState<Omit<RelationType, 'id' | 'nodeName' | 'path' | 'created_at' | 'updated_at'>>({
    relation_name: '',
    title: '',
    description: '',
    category: 'social',
    inverse_relation_name: '',
    bidirectional: false,
    implies_stewardship: false,
    requires_minor: false,
    icon: 'link-2',
    color: '#3b82f6',
  })
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (editingRelationType) {
      setFormData({
        relation_name: editingRelationType.relation_name,
        title: editingRelationType.title,
        description: editingRelationType.description || '',
        category: editingRelationType.category,
        inverse_relation_name: editingRelationType.inverse_relation_name || '',
        bidirectional: editingRelationType.bidirectional,
        implies_stewardship: editingRelationType.implies_stewardship,
        requires_minor: editingRelationType.requires_minor,
        icon: editingRelationType.icon || 'link-2',
        color: editingRelationType.color || '#3b82f6',
      })
    } else {
      setFormData({
        relation_name: '',
        title: '',
        description: '',
        category: 'social',
        inverse_relation_name: '',
        bidirectional: false,
        implies_stewardship: false,
        requires_minor: false,
        icon: 'link-2',
        color: '#3b82f6',
      })
    }
    setError(null)
  }, [editingRelationType, isOpen])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)

    // Validation
    if (!formData.relation_name.trim()) {
      setError('Relation name is required')
      return
    }
    if (!formData.title.trim()) {
      setError('Title is required')
      return
    }

    // Convert relation_name to UPPER_SNAKE_CASE
    const relation_name = formData.relation_name
      .trim()
      .toUpperCase()
      .replace(/\s+/g, '_')
      .replace(/[^A-Z0-9_]/g, '')

    setSaving(true)
    try {
      await onSave({
        ...formData,
        relation_name,
        title: formData.title.trim(),
        description: formData.description?.trim() || undefined,
      })
      onClose()
    } catch (err: any) {
      setError(err.message || 'Failed to save relation type')
    } finally {
      setSaving(false)
    }
  }

  if (!isOpen) return null

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="dialog-title"
    >
      <div
        className="bg-zinc-900/90 backdrop-blur-xl border border-white/20 shadow-2xl rounded-2xl max-w-2xl w-full max-h-[90vh] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <form onSubmit={handleSubmit}>
          <div className="sticky top-0 bg-zinc-900/95 backdrop-blur-md border-b border-white/10 px-6 py-4 flex items-center justify-between">
            <h2 id="dialog-title" className="text-2xl font-bold text-white flex items-center gap-2">
              <Link2 className="w-6 h-6 text-primary-400" />
              {editingRelationType ? 'Edit Relation Type' : 'Create Relation Type'}
            </h2>
            <button
              type="button"
              onClick={onClose}
              className="p-2 hover:bg-white/10 rounded-lg transition-colors text-zinc-400 hover:text-white"
              aria-label="Close dialog"
            >
              <X className="w-5 h-5" />
            </button>
          </div>

          <div className="p-6 space-y-6">
            {error && (
              <div className="p-4 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300 text-sm">
                {error}
              </div>
            )}

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <label htmlFor="relation_name" className="block text-sm font-medium text-zinc-300">
                  Relation Name <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  id="relation_name"
                  required
                  disabled={!!editingRelationType}
                  value={formData.relation_name}
                  onChange={(e) => setFormData({ ...formData, relation_name: e.target.value })}
                  className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20 disabled:cursor-not-allowed disabled:opacity-50"
                  placeholder="PARENT_OF"
                />
                <p className="text-xs text-zinc-500">
                  Will be converted to UPPER_SNAKE_CASE
                </p>
              </div>

              <div className="space-y-2">
                <label htmlFor="title" className="block text-sm font-medium text-zinc-300">
                  Display Name <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  id="title"
                  required
                  value={formData.title}
                  onChange={(e) => setFormData({ ...formData, title: e.target.value })}
                  className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                  placeholder="Parent Of"
                />
              </div>
            </div>

            <div className="space-y-2">
              <label htmlFor="description" className="block text-sm font-medium text-zinc-300">
                Description
              </label>
              <textarea
                id="description"
                value={formData.description}
                onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                rows={3}
                className="w-full resize-none rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                placeholder="Describes a parent-child relationship"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <label htmlFor="category" className="block text-sm font-medium text-zinc-300">
                  Category <span className="text-red-400">*</span>
                </label>
                <select
                  id="category"
                  required
                  value={formData.category}
                  onChange={(e) => setFormData({ ...formData, category: e.target.value as 'household' | 'organization' | 'social' })}
                  className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                >
                  <option value="household">Household</option>
                  <option value="organization">Organization</option>
                  <option value="social">Social</option>
                </select>
              </div>

              <div className="space-y-2">
                <label htmlFor="inverse_relation_name" className="block text-sm font-medium text-zinc-300">
                  Inverse Relation Name
                </label>
                <input
                  type="text"
                  id="inverse_relation_name"
                  value={formData.inverse_relation_name}
                  onChange={(e) => setFormData({ ...formData, inverse_relation_name: e.target.value })}
                  className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                  placeholder="CHILD_OF"
                />
              </div>
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <label htmlFor="icon" className="block text-sm font-medium text-zinc-300">
                  Icon
                </label>
                <input
                  type="text"
                  id="icon"
                  value={formData.icon}
                  onChange={(e) => setFormData({ ...formData, icon: e.target.value })}
                  className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                  placeholder="link-2"
                />
                <p className="text-xs text-zinc-500">Lucide icon name</p>
              </div>

              <div className="space-y-2">
                <label htmlFor="color" className="block text-sm font-medium text-zinc-300">
                  Color
                </label>
                <div className="flex gap-2">
                  <input
                    type="color"
                    id="color"
                    value={formData.color}
                    onChange={(e) => setFormData({ ...formData, color: e.target.value })}
                    className="w-16 h-10 rounded-lg border border-white/10 bg-white/5 cursor-pointer"
                  />
                  <input
                    type="text"
                    value={formData.color}
                    onChange={(e) => setFormData({ ...formData, color: e.target.value })}
                    className="flex-1 rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                    placeholder="#3b82f6"
                  />
                </div>
              </div>
            </div>

            <div className="space-y-3 border-t border-white/10 pt-4">
              <label className="flex items-center gap-3 p-3 rounded-lg hover:bg-white/5 cursor-pointer transition-colors">
                <input
                  type="checkbox"
                  checked={formData.bidirectional}
                  onChange={(e) => setFormData({ ...formData, bidirectional: e.target.checked })}
                  className="w-5 h-5 rounded border-white/10 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/20"
                />
                <div className="flex-1">
                  <div className="text-sm font-medium text-white">Bidirectional</div>
                  <div className="text-xs text-zinc-400">Relation applies in both directions</div>
                </div>
              </label>

              <label className="flex items-center gap-3 p-3 rounded-lg hover:bg-white/5 cursor-pointer transition-colors">
                <input
                  type="checkbox"
                  checked={formData.implies_stewardship}
                  onChange={(e) => {
                    const checked = e.target.checked
                    setFormData({
                      ...formData,
                      implies_stewardship: checked,
                      requires_minor: checked ? formData.requires_minor : false,
                    })
                  }}
                  className="w-5 h-5 rounded border-white/10 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/20"
                />
                <div className="flex-1">
                  <div className="text-sm font-medium text-white">Implies Stewardship</div>
                  <div className="text-xs text-zinc-400">This relation grants stewardship permissions</div>
                </div>
              </label>

              {formData.implies_stewardship && (
                <label className="flex items-center gap-3 p-3 ml-8 rounded-lg hover:bg-white/5 cursor-pointer transition-colors border-l-2 border-primary-500/30">
                  <input
                    type="checkbox"
                    checked={formData.requires_minor}
                    onChange={(e) => setFormData({ ...formData, requires_minor: e.target.checked })}
                    className="w-5 h-5 rounded border-white/10 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/20"
                  />
                  <div className="flex-1">
                    <div className="text-sm font-medium text-white">Requires Minor</div>
                    <div className="text-xs text-zinc-400">Target must be marked as a minor</div>
                  </div>
                </label>
              )}
            </div>
          </div>

          <div className="sticky bottom-0 bg-zinc-900/95 backdrop-blur-md border-t border-white/10 px-6 py-4 flex items-center justify-end gap-3">
            <button
              type="button"
              onClick={onClose}
              disabled={saving}
              className="px-6 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-6 py-2 rounded-lg bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
            >
              {saving ? 'Saving...' : editingRelationType ? 'Update' : 'Create'}
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body
  )
}

export default function RelationTypesManager() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'

  const [relationTypes, setRelationTypes] = useState<RelationType[]>([])
  const [filteredRelationTypes, setFilteredRelationTypes] = useState<RelationType[]>([])
  const [loading, setLoading] = useState(true)
  const [searchTerm, setSearchTerm] = useState('')
  const [categoryFilter, setCategoryFilter] = useState<'all' | 'household' | 'organization' | 'social'>('all')
  const [showDialog, setShowDialog] = useState(false)
  const [editingRelationType, setEditingRelationType] = useState<RelationType | undefined>(undefined)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadRelationTypes()
  }, [repo, activeBranch])

  useEffect(() => {
    filterRelationTypes()
  }, [relationTypes, searchTerm, categoryFilter])

  async function loadRelationTypes() {
    if (!repo) return
    setLoading(true)

    try {
      const nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, BASE_PATH)
      const relationTypeNodes = nodes.filter(n => n.node_type === 'raisin:RelationType')
      setRelationTypes(relationTypeNodes.map(nodeToRelationType))
    } catch (error) {
      console.error('Failed to load relation types:', error)
      showError('Load Failed', 'Failed to load relation types')
    } finally {
      setLoading(false)
    }
  }

  function filterRelationTypes() {
    let filtered = relationTypes

    if (searchTerm) {
      const term = searchTerm.toLowerCase()
      filtered = filtered.filter(rt =>
        rt.relation_name.toLowerCase().includes(term) ||
        rt.title.toLowerCase().includes(term) ||
        rt.description?.toLowerCase().includes(term)
      )
    }

    if (categoryFilter !== 'all') {
      filtered = filtered.filter(rt => rt.category === categoryFilter)
    }

    setFilteredRelationTypes(filtered)
  }

  async function handleSave(data: Omit<RelationType, 'id' | 'nodeName' | 'path' | 'created_at' | 'updated_at'>) {
    if (!repo) return

    const properties = {
      relation_name: data.relation_name,
      title: data.title,
      description: data.description || undefined,
      category: data.category,
      inverse_relation_name: data.inverse_relation_name || undefined,
      bidirectional: data.bidirectional,
      implies_stewardship: data.implies_stewardship,
      requires_minor: data.requires_minor,
      icon: data.icon || undefined,
      color: data.color || undefined,
    }

    const commit = {
      message: editingRelationType
        ? `Update relation type: ${data.relation_name}`
        : `Create relation type: ${data.relation_name}`,
      actor: 'admin',
    }

    try {
      if (editingRelationType) {
        const request: UpdateNodeRequest = {
          properties,
          commit,
        }
        await nodesApi.update(repo, activeBranch, WORKSPACE, editingRelationType.path, request)
        showSuccess('Updated', 'Relation type updated successfully')
      } else {
        const request: CreateNodeRequest = {
          name: data.relation_name,
          node_type: 'raisin:RelationType',
          properties,
          commit,
        }
        await nodesApi.create(repo, activeBranch, WORKSPACE, BASE_PATH, request)
        showSuccess('Created', 'Relation type created successfully')
      }

      setShowDialog(false)
      setEditingRelationType(undefined)
      loadRelationTypes()
    } catch (error: any) {
      throw new Error(error.message || 'Failed to save relation type')
    }
  }

  async function handleDelete(relationType: RelationType) {
    if (!repo) return

    setDeleteConfirm({
      message: `Are you sure you want to delete relation type "${relationType.title}"?`,
      onConfirm: async () => {
        try {
          await nodesApi.delete(repo, activeBranch, WORKSPACE, relationType.path)
          showSuccess('Deleted', 'Relation type deleted successfully')
          loadRelationTypes()
        } catch (error) {
          console.error('Failed to delete relation type:', error)
          showError('Delete Failed', 'Failed to delete relation type')
        }
      },
    })
  }

  function handleEdit(relationType: RelationType) {
    setEditingRelationType(relationType)
    setShowDialog(true)
  }

  function handleCreate() {
    setEditingRelationType(undefined)
    setShowDialog(true)
  }

  const relationTypeColumns: TableColumn<RelationType>[] = [
    {
      key: 'relation_name',
      header: 'Type ID',
      render: (rt) => (
        <div className="flex items-center gap-2">
          <Link2 className="w-4 h-4 text-primary-400" />
          <span className="text-white font-mono font-medium">{rt.relation_name}</span>
        </div>
      ),
    },
    {
      key: 'title',
      header: 'Display Name',
      render: (rt) => <span className="text-zinc-300">{rt.title}</span>,
    },
    {
      key: 'category',
      header: 'Category',
      render: (rt) => {
        const colors = {
          household: 'bg-green-500/20 text-green-300',
          organization: 'bg-blue-500/20 text-blue-300',
          social: 'bg-purple-500/20 text-purple-300',
        }
        return (
          <span className={`px-2 py-0.5 rounded-full text-xs ${colors[rt.category]}`}>
            {rt.category}
          </span>
        )
      },
    },
    {
      key: 'implies_stewardship',
      header: 'Stewardship',
      render: (rt) => rt.implies_stewardship ? (
        <span className="px-2 py-0.5 bg-amber-500/20 text-amber-300 text-xs rounded-full">
          Yes
        </span>
      ) : (
        <span className="text-zinc-500 text-xs">No</span>
      ),
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex flex-col md:flex-row justify-between items-start gap-4">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <h1 className="text-4xl font-bold text-white">Relation Types</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage relationship types for stewardship system</p>
        </div>
        <div className="flex gap-2 w-full md:w-auto">
          <button
            onClick={handleCreate}
            className="flex-1 md:flex-none flex items-center justify-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Plus className="w-5 h-5" />
            <span className="md:inline">New Relation Type</span>
          </button>
        </div>
      </div>

      <div className="mb-6 flex flex-col md:flex-row gap-4">
        <div className="flex-1 relative">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-zinc-400" />
          <input
            type="text"
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            placeholder="Search relation types..."
            className="w-full pl-10 pr-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
          />
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => setCategoryFilter('all')}
            className={`px-4 py-2 rounded-lg transition-colors ${
              categoryFilter === 'all'
                ? 'bg-primary-500 text-white'
                : 'bg-white/5 text-zinc-400 hover:bg-white/10'
            }`}
          >
            All
          </button>
          <button
            onClick={() => setCategoryFilter('household')}
            className={`px-4 py-2 rounded-lg transition-colors ${
              categoryFilter === 'household'
                ? 'bg-green-500 text-white'
                : 'bg-white/5 text-zinc-400 hover:bg-white/10'
            }`}
          >
            Household
          </button>
          <button
            onClick={() => setCategoryFilter('organization')}
            className={`px-4 py-2 rounded-lg transition-colors ${
              categoryFilter === 'organization'
                ? 'bg-blue-500 text-white'
                : 'bg-white/5 text-zinc-400 hover:bg-white/10'
            }`}
          >
            Organization
          </button>
          <button
            onClick={() => setCategoryFilter('social')}
            className={`px-4 py-2 rounded-lg transition-colors ${
              categoryFilter === 'social'
                ? 'bg-purple-500 text-white'
                : 'bg-white/5 text-zinc-400 hover:bg-white/10'
            }`}
          >
            Social
          </button>
        </div>
      </div>

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading...</div>
      ) : filteredRelationTypes.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Link2 className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">
              {relationTypes.length === 0 ? 'No relation types yet' : 'No matching relation types'}
            </h3>
            <p className="text-zinc-400">
              {relationTypes.length === 0
                ? 'Create a relation type to get started'
                : 'Try adjusting your search or filters'}
            </p>
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="overflow-hidden">
          <ItemTable
            items={filteredRelationTypes}
            columns={relationTypeColumns}
            getItemId={(rt) => rt.id || rt.relation_name}
            getItemPath={(rt) => rt.path}
            getItemName={(rt) => rt.title}
            itemType="relation-type"
            onEdit={handleEdit}
            onDelete={handleDelete}
            onReorder={undefined}
          />
        </GlassCard>
      )}

      <RelationTypeDialog
        isOpen={showDialog}
        onClose={() => {
          setShowDialog(false)
          setEditingRelationType(undefined)
        }}
        onSave={handleSave}
        editingRelationType={editingRelationType}
      />

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

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
