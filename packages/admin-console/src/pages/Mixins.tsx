import { useEffect, useState, useMemo } from 'react'
import { Link, useParams } from 'react-router-dom'
import { Layers, Plus, CheckCircle, XCircle, History, Search, X } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import { mixinsApi, type Mixin } from '../api/mixins'

export default function Mixins() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'
  const [mixins, setMixins] = useState<Mixin[]>([])
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedNamespace, setSelectedNamespace] = useState<string | null>(null)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  // Extract unique namespaces from mixin names
  const namespaces = useMemo(() => {
    const nsSet = new Set<string>()
    mixins.forEach((m) => {
      const colonIndex = m.name.indexOf(':')
      if (colonIndex > 0) {
        nsSet.add(m.name.substring(0, colonIndex))
      }
    })
    return Array.from(nsSet).sort()
  }, [mixins])

  const filteredMixins = useMemo(() => {
    return mixins.filter((m) => {
      const matchesSearch = searchQuery === '' || m.name.toLowerCase().includes(searchQuery.toLowerCase())
      const matchesNamespace = selectedNamespace === null || m.name.startsWith(selectedNamespace + ':')
      return matchesSearch && matchesNamespace
    })
  }, [mixins, searchQuery, selectedNamespace])

  useEffect(() => {
    loadMixins()
  }, [repo, activeBranch])

  async function loadMixins() {
    if (!repo) return
    setLoading(true)
    try {
      const data = await mixinsApi.list(repo, activeBranch)
      setMixins(data)
    } catch (error) {
      console.error('Failed to load mixins:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(mixin: Mixin) {
    if (!repo) return
    setDeleteConfirm({
      message: `Are you sure you want to delete "${mixin.name}"?`,
      onConfirm: async () => {
        try {
          await mixinsApi.delete(repo, activeBranch, mixin.name)
          loadMixins()
          showSuccess('Deleted', 'Mixin deleted successfully')
        } catch (error) {
          console.error('Failed to delete mixin:', error)
          showError('Delete Failed', 'Failed to delete mixin')
        }
      }
    })
  }

  const mixinColumns: TableColumn<Mixin>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (mixin) => (
        <div className="flex items-center gap-2">
          <Layers className="w-4 h-4 text-primary-400" />
          <span className="text-white font-medium">{mixin.name}</span>
        </div>
      ),
    },
    {
      key: 'properties',
      header: 'Properties',
      width: '120px',
      render: (mixin) => (
        <span className="text-primary-300">
          {Array.isArray(mixin.properties) ? mixin.properties.length : 0}
        </span>
      ),
    },
    {
      key: 'version',
      header: 'Version',
      width: '100px',
      render: (mixin) => (
        mixin.version ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 text-blue-400 text-xs rounded-full w-fit">
            <History className="w-3 h-3" />
            v{mixin.version}
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
      render: (mixin) => (
        mixin.published ? (
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
      render: (mixin) => (
        <span className="text-zinc-400 text-sm">
          {mixin.updated_at ? new Date(mixin.updated_at).toLocaleDateString() : '-'}
        </span>
      ),
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex justify-between items-start">
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">Mixins</h1>
          <p className="text-zinc-400">Reusable property sets composed into node types</p>
        </div>
        <Link
          to={`/${repo}/${activeBranch}/mixins/new`}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" />
          New Mixin
        </Link>
      </div>

      {/* Search and Namespace Filters */}
      <div className="mb-4 flex flex-col gap-3">
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
      ) : mixins.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Layers className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No mixins yet</h3>
            <p className="text-zinc-400">Create a mixin to share properties across node types</p>
          </div>
        </GlassCard>
      ) : filteredMixins.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Search className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No matching mixins</h3>
            <p className="text-zinc-400">Try adjusting your search or filter</p>
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="flex-1 overflow-hidden flex flex-col">
          <ItemTable
            items={filteredMixins}
            columns={mixinColumns}
            getItemId={(m) => m.name}
            getItemPath={(m) => m.name}
            getItemName={(m) => m.name}
            itemType="mixin"
            editPath={(m) => `/${repo}/${activeBranch}/mixins/${m.name}`}
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
