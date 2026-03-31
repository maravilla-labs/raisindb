import { useEffect, useState, useMemo } from 'react'
import { Link, useParams } from 'react-router-dom'
import { FileType, Plus, CheckCircle, XCircle, History, Search, X } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import { nodeTypesApi, type NodeType } from '../api/nodetypes'

export default function NodeTypes() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'
  const [nodeTypes, setNodeTypes] = useState<NodeType[]>([])
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedNamespace, setSelectedNamespace] = useState<string | null>(null)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  // Extract unique namespaces from node type names
  const namespaces = useMemo(() => {
    const nsSet = new Set<string>()
    nodeTypes.forEach((nt) => {
      const colonIndex = nt.name.indexOf(':')
      if (colonIndex > 0) {
        nsSet.add(nt.name.substring(0, colonIndex))
      }
    })
    return Array.from(nsSet).sort()
  }, [nodeTypes])

  // Filter node types by search query and namespace
  const filteredNodeTypes = useMemo(() => {
    return nodeTypes.filter((nt) => {
      const matchesSearch = searchQuery === '' || nt.name.toLowerCase().includes(searchQuery.toLowerCase())
      const matchesNamespace = selectedNamespace === null || nt.name.startsWith(selectedNamespace + ':')
      return matchesSearch && matchesNamespace
    })
  }, [nodeTypes, searchQuery, selectedNamespace])

  useEffect(() => {
    loadNodeTypes()
  }, [repo, activeBranch])

  async function loadNodeTypes() {
    if (!repo) return
    setLoading(true)
    try {
      const data = await nodeTypesApi.list(repo, activeBranch)
      setNodeTypes(data)
    } catch (error) {
      console.error('Failed to load node types:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(nodeType: NodeType) {
    if (!repo) return
    setDeleteConfirm({
      message: `Are you sure you want to delete "${nodeType.name}"?`,
      onConfirm: async () => {
        try {
          await nodeTypesApi.delete(repo, activeBranch, nodeType.name)
          loadNodeTypes()
          showSuccess('Deleted', 'Node type deleted successfully')
        } catch (error) {
          console.error('Failed to delete node type:', error)
          showError('Delete Failed', 'Failed to delete node type')
        }
      }
    })
  }

  // Define table columns for node types
  const nodeTypeColumns: TableColumn<NodeType>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (nodeType) => (
        <div className="flex items-center gap-2">
          <FileType className="w-4 h-4 text-primary-400" />
          <span className="text-white font-medium">{nodeType.name}</span>
        </div>
      ),
    },
    {
      key: 'extends',
      header: 'Extends',
      render: (nodeType) => (
        <span className="text-primary-300">{nodeType.extends || '-'}</span>
      ),
    },
    {
      key: 'version',
      header: 'Version',
      width: '100px',
      render: (nodeType) => (
        nodeType.version ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 text-blue-400 text-xs rounded-full w-fit">
            <History className="w-3 h-3" />
            v{nodeType.version}
          </span>
        ) : (
          <span className="text-zinc-500">-</span>
        )
      ),
    },
    {
      key: 'status',
      header: 'Status',
      width: '120px',
      render: (nodeType) => (
        nodeType.published ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-400 text-xs rounded-full w-fit">
            <CheckCircle className="w-3 h-3" />
            Published
          </span>
        ) : (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-gray-500/20 text-zinc-400 text-xs rounded-full w-fit">
            <XCircle className="w-3 h-3" />
            Draft
          </span>
        )
      ),
    },
    {
      key: 'updated',
      header: 'Updated',
      width: '140px',
      render: (nodeType) => (
        <span className="text-zinc-400 text-sm">
          {nodeType.updated_at ? new Date(nodeType.updated_at).toLocaleDateString() : '-'}
        </span>
      ),
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex justify-between items-start">
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">Node Types</h1>
          <p className="text-zinc-400">Define and manage content types</p>
        </div>
        <Link
          to={`/${repo}/${activeBranch}/nodetypes/new`}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" />
          New Node Type
        </Link>
      </div>

      {/* Search and Namespace Filters */}
      <div className="mb-4 flex flex-col gap-3">
        {/* Search Input */}
        <div className="relative max-w-md">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-400" />
          <input
            type="text"
            placeholder="Search by name..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-10 pr-10 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:outline-none focus:border-primary-400 focus:ring-1 focus:ring-primary-400"
          />
          {searchQuery && (
            <button
              onClick={() => setSearchQuery('')}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-zinc-400 hover:text-white"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Namespace Filter Tags */}
        {namespaces.length > 0 && (
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => setSelectedNamespace(null)}
              className={`px-3 py-1 text-sm rounded-full transition-all ${
                selectedNamespace === null
                  ? 'bg-primary-500 text-white'
                  : 'bg-white/10 text-zinc-400 hover:bg-white/20'
              }`}
            >
              All
            </button>
            {namespaces.map((ns) => (
              <button
                key={ns}
                onClick={() => setSelectedNamespace(selectedNamespace === ns ? null : ns)}
                className={`px-3 py-1 text-sm rounded-full transition-all ${
                  selectedNamespace === ns
                    ? 'bg-primary-500 text-white'
                    : 'bg-white/10 text-zinc-400 hover:bg-white/20'
                }`}
              >
                {ns}
              </button>
            ))}
          </div>
        )}
      </div>

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading...</div>
      ) : nodeTypes.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <FileType className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No node types yet</h3>
            <p className="text-zinc-400">Create your first node type to get started</p>
          </div>
        </GlassCard>
      ) : filteredNodeTypes.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Search className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No matching node types</h3>
            <p className="text-zinc-400">Try adjusting your search or filter</p>
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="flex-1 overflow-hidden flex flex-col">
          <ItemTable
            items={filteredNodeTypes}
            columns={nodeTypeColumns}
            getItemId={(nt) => nt.name}
            getItemPath={(nt) => nt.name}
            getItemName={(nt) => nt.name}
            itemType="nodetype"
            editPath={(nt) => `/${repo}/${activeBranch}/nodetypes/${nt.name}`}
            onDelete={handleDelete}
          />
        </GlassCard>
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
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
