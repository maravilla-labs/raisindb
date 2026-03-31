import { useEffect, useState, useMemo } from 'react'
import { Link, useParams } from 'react-router-dom'
import { Puzzle, Plus, CheckCircle, XCircle, History, Search, X } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import { archetypesApi, type Archetype } from '../api/archetypes'

export default function Archetypes() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'
  const [archetypes, setArchetypes] = useState<Archetype[]>([])
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedNamespace, setSelectedNamespace] = useState<string | null>(null)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  // Extract unique namespaces from archetype names
  const namespaces = useMemo(() => {
    const nsSet = new Set<string>()
    archetypes.forEach((a) => {
      const colonIndex = a.name.indexOf(':')
      if (colonIndex > 0) {
        nsSet.add(a.name.substring(0, colonIndex))
      }
    })
    return Array.from(nsSet).sort()
  }, [archetypes])

  // Filter archetypes by search query and namespace
  const filteredArchetypes = useMemo(() => {
    return archetypes.filter((a) => {
      const matchesSearch = searchQuery === '' || a.name.toLowerCase().includes(searchQuery.toLowerCase())
      const matchesNamespace = selectedNamespace === null || a.name.startsWith(selectedNamespace + ':')
      return matchesSearch && matchesNamespace
    })
  }, [archetypes, searchQuery, selectedNamespace])

  useEffect(() => {
    loadArchetypes()
  }, [repo, activeBranch])

  async function loadArchetypes() {
    if (!repo) return
    setLoading(true)
    try {
      const data = await archetypesApi.list(repo, activeBranch)
      setArchetypes(data)
    } catch (error) {
      console.error('Failed to load archetypes:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(archetype: Archetype) {
    if (!repo) return
    setDeleteConfirm({
      message: `Are you sure you want to delete archetype "${archetype.name}"?`,
      onConfirm: async () => {
        try {
          await archetypesApi.delete(repo, activeBranch, archetype.name)
          loadArchetypes()
          showSuccess('Deleted', 'Archetype deleted successfully')
        } catch (error) {
          console.error('Failed to delete archetype:', error)
          showError('Delete Failed', 'Failed to delete archetype')
        }
      }
    })
  }

  // Define table columns for archetypes
  const archetypeColumns: TableColumn<Archetype>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (archetype) => (
        <div className="flex items-center gap-2">
          <Puzzle className="w-4 h-4 text-primary-400" />
          <div>
            <span className="text-white font-medium">{archetype.name}</span>
            {archetype.title && (
              <span className="text-zinc-400 text-sm ml-2">({archetype.title})</span>
            )}
          </div>
        </div>
      ),
    },
    {
      key: 'base_node_type',
      header: 'Base Type',
      render: (archetype) => (
        <span className="text-primary-300">{archetype.base_node_type || '-'}</span>
      ),
    },
    {
      key: 'description',
      header: 'Description',
      render: (archetype) => (
        <span className="text-zinc-300 text-sm line-clamp-1">{archetype.description || '-'}</span>
      ),
    },
    {
      key: 'version',
      header: 'Version',
      width: '100px',
      render: (archetype) => (
        archetype.version ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 text-blue-400 text-xs rounded-full w-fit">
            <History className="w-3 h-3" />
            v{archetype.version}
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
      render: (archetype) => {
        const isPublished = archetype.publishable ?? false
        return isPublished ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-400 text-xs rounded-full w-fit">
            <CheckCircle className="w-3 h-3" />
            Published
          </span>
        ) : (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-slate-500/20 text-zinc-400 text-xs rounded-full w-fit">
            <XCircle className="w-3 h-3" />
            Draft
          </span>
        )
      },
    },
    {
      key: 'updated',
      header: 'Updated',
      width: '140px',
      render: (archetype) => (
        <span className="text-zinc-400 text-sm">
          {archetype.updated_at ? new Date(archetype.updated_at).toLocaleDateString() : '-'}
        </span>
      ),
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex justify-between items-start">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <h1 className="text-4xl font-bold text-white">Archetypes</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage reusable archetype definitions</p>
        </div>
        <Link
          to={`/${repo}/${activeBranch}/archetypes/new`}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" />
          New Archetype
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
        <div className="text-center text-zinc-400 py-12">Loading archetypes...</div>
      ) : archetypes.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Puzzle className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No archetypes yet</h3>
            <p className="text-zinc-400">Archetypes help you preset content structures and views</p>
          </div>
        </GlassCard>
      ) : filteredArchetypes.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Search className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No matching archetypes</h3>
            <p className="text-zinc-400">Try adjusting your search or filter</p>
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="flex-1 overflow-hidden flex flex-col">
          <ItemTable
            items={filteredArchetypes}
            columns={archetypeColumns}
            getItemId={(a) => a.name}
            getItemPath={(a) => a.name}
            getItemName={(a) => a.name}
            itemType="archetype"
            editPath={(a) => `/${repo}/${activeBranch}/archetypes/${a.name}`}
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
