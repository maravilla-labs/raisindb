import { useEffect, useState, useMemo } from 'react'
import { Link, useParams } from 'react-router-dom'
import { Shapes, CheckCircle, XCircle, Plus, History, Search, X } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ItemTable, type TableColumn } from '../components/ItemTable'
import { useToast, ToastContainer } from '../components/Toast'
import { elementTypesApi, type ElementType } from '../api/elementtypes'

export default function ElementTypes() {
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'
  const [elementTypes, setElementTypes] = useState<ElementType[]>([])
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedNamespace, setSelectedNamespace] = useState<string | null>(null)
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  // Extract unique namespaces from element type names
  const namespaces = useMemo(() => {
    const nsSet = new Set<string>()
    elementTypes.forEach((et) => {
      const colonIndex = et.name.indexOf(':')
      if (colonIndex > 0) {
        nsSet.add(et.name.substring(0, colonIndex))
      }
    })
    return Array.from(nsSet).sort()
  }, [elementTypes])

  // Filter element types by search query and namespace
  const filteredElementTypes = useMemo(() => {
    return elementTypes.filter((et) => {
      const matchesSearch = searchQuery === '' || et.name.toLowerCase().includes(searchQuery.toLowerCase())
      const matchesNamespace = selectedNamespace === null || et.name.startsWith(selectedNamespace + ':')
      return matchesSearch && matchesNamespace
    })
  }, [elementTypes, searchQuery, selectedNamespace])

  useEffect(() => {
    loadElementTypes()
  }, [repo, activeBranch])

  async function loadElementTypes() {
    if (!repo) return
    setLoading(true)
    try {
      const data = await elementTypesApi.list(repo, activeBranch)
      setElementTypes(data)
    } catch (error) {
      console.error('Failed to load element types:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(elementType: ElementType) {
    if (!repo) return
    setDeleteConfirm({
      message: `Are you sure you want to delete element type "${elementType.name}"?`,
      onConfirm: async () => {
        try {
          await elementTypesApi.delete(repo, activeBranch, elementType.name)
          loadElementTypes()
          showSuccess('Deleted', 'Element type deleted successfully')
        } catch (error) {
          console.error('Failed to delete element type:', error)
          showError('Delete Failed', 'Failed to delete element type')
        }
      }
    })
  }

  // Define table columns for element types
  const elementTypeColumns: TableColumn<ElementType>[] = [
    {
      key: 'name',
      header: 'Name',
      render: (elementType) => (
        <div className="flex items-center gap-2">
          <Shapes className="w-4 h-4 text-primary-400" />
          <div>
            <span className="text-white font-medium">{elementType.name}</span>
            {elementType.title && (
              <span className="text-zinc-400 text-sm ml-2">({elementType.title})</span>
            )}
          </div>
        </div>
      ),
    },
    {
      key: 'description',
      header: 'Description',
      render: (elementType) => (
        <span className="text-zinc-300 text-sm line-clamp-1">{elementType.description || '-'}</span>
      ),
    },
    {
      key: 'fields',
      header: 'Fields',
      width: '80px',
      render: (elementType) => (
        <span className="text-primary-300">{elementType.fields?.length ?? 0}</span>
      ),
    },
    {
      key: 'version',
      header: 'Version',
      width: '100px',
      render: (elementType) => (
        elementType.version ? (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 text-blue-400 text-xs rounded-full w-fit">
            <History className="w-3 h-3" />
            v{elementType.version}
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
      render: (elementType) => {
        const isPublished = elementType.publishable ?? false
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
      render: (elementType) => (
        <span className="text-zinc-400 text-sm">
          {elementType.updated_at ? new Date(elementType.updated_at).toLocaleDateString() : '-'}
        </span>
      ),
    },
  ]

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex justify-between items-start">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <h1 className="text-4xl font-bold text-white">Element Types</h1>
            <span className="px-2 py-1 bg-amber-500/20 border border-amber-400/30 rounded text-amber-300 text-sm font-medium">
              Experimental
            </span>
          </div>
          <p className="text-zinc-400">Manage reusable element building blocks</p>
        </div>
        <Link
          to={`/${repo}/${activeBranch}/elementtypes/new`}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" />
          New Element Type
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
        <div className="text-center text-zinc-400 py-12">Loading element types...</div>
      ) : elementTypes.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Shapes className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No element types yet</h3>
            <p className="text-zinc-400">
              Element types define structured blocks of content for composites and elements
            </p>
          </div>
        </GlassCard>
      ) : filteredElementTypes.length === 0 ? (
        <GlassCard className="h-full">
          <div className="text-center py-12">
            <Search className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No matching element types</h3>
            <p className="text-zinc-400">Try adjusting your search or filter</p>
          </div>
        </GlassCard>
      ) : (
        <GlassCard className="flex-1 overflow-hidden flex flex-col">
          <ItemTable
            items={filteredElementTypes}
            columns={elementTypeColumns}
            getItemId={(et) => et.name}
            getItemPath={(et) => et.name}
            getItemName={(et) => et.name}
            itemType="elementtype"
            editPath={(et) => `/${repo}/${activeBranch}/elementtypes/${et.name}`}
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
