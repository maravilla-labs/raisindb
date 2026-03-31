import { useState, useEffect, useMemo } from 'react'
import { useParams } from 'react-router-dom'
import { Settings2, Plus, Trash2, GripVertical, ChevronDown, ChevronRight, Loader2 } from 'lucide-react'
import NodeTypePicker from '../shared/NodeTypePicker'
import { nodeTypesApi, type NodeType } from '../../api/nodetypes'
import type { NodeTypeDefinition, IndexType, CompoundIndexDefinition, CompoundIndexColumn } from './types'

interface CoreSettingsPanelProps {
  nodeType: NodeTypeDefinition
  onChange: (nodeType: NodeTypeDefinition) => void
  validationErrors: Record<string, string>
}

// Resolved property from a mixin, with source attribution
interface MixinProperty {
  name: string
  type: string
  source: string // mixin name it came from
  required?: boolean
}

export default function CoreSettingsPanel({
  nodeType,
  onChange,
  validationErrors,
}: CoreSettingsPanelProps) {
  const { repo, branch: branchParam } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branchParam || 'main'

  // Mixin definitions cache and fetching
  const [mixinDefs, setMixinDefs] = useState<Map<string, NodeType>>(new Map())
  const [mixinLoading, setMixinLoading] = useState(false)
  const [mixinExpanded, setMixinExpanded] = useState(true)

  const mixins = nodeType.mixins || []

  useEffect(() => {
    if (!repo || mixins.length === 0) {
      setMixinDefs(new Map())
      return
    }

    let cancelled = false

    async function fetchMixins() {
      setMixinLoading(true)
      const newDefs = new Map<string, NodeType>()

      await Promise.all(
        mixins.map(async (name) => {
          try {
            const def = await nodeTypesApi.get(repo!, activeBranch, name)
            if (!cancelled) {
              newDefs.set(name, def)
            }
          } catch (err) {
            console.error(`Failed to fetch mixin ${name}:`, err)
          }
        })
      )

      if (!cancelled) {
        setMixinDefs(newDefs)
        setMixinLoading(false)
      }
    }

    fetchMixins()
    return () => { cancelled = true }
  }, [repo, activeBranch, mixins.join(',')])

  // Resolve inherited properties from all selected mixins
  const mixinProperties = useMemo((): MixinProperty[] => {
    const props: MixinProperty[] = []
    for (const name of mixins) {
      const def = mixinDefs.get(name)
      if (!def?.properties) continue
      const properties = Array.isArray(def.properties) ? def.properties : []
      for (const p of properties) {
        if (p.name) {
          props.push({
            name: p.name,
            type: p.type || 'Unknown',
            source: name,
            required: p.required,
          })
        }
      }
    }
    return props
  }, [mixins, mixinDefs])

  const updateNodeType = (updates: Partial<NodeTypeDefinition>) => {
    onChange({ ...nodeType, ...updates })
  }

  const updateIndexTypes = (enabled: boolean, indexType: IndexType) => {
    // If index_types is undefined, we're starting from the "all enabled" default state
    const allTypes: IndexType[] = ['Fulltext', 'Vector', 'Property']
    const current = nodeType.index_types || allTypes

    const updated = enabled
      ? [...current, indexType].filter((v, i, a) => a.indexOf(v) === i) // Add and dedupe
      : current.filter((t) => t !== indexType) // Remove

    // If all three are selected, we can set to undefined (matches backend default)
    // Otherwise, keep the explicit array
    const hasAllTypes = allTypes.every((t) => updated.includes(t))

    updateNodeType({
      index_types: hasAllTypes ? undefined : updated.length > 0 ? updated : undefined,
    })
  }

  // Compound index management
  const addCompoundIndex = () => {
    const newIndex: CompoundIndexDefinition = {
      name: `idx_${Date.now()}`,
      columns: [{ property: '__node_type' }],
      has_order_column: true,
    }
    updateNodeType({
      compound_indexes: [...(nodeType.compound_indexes || []), newIndex],
    })
  }

  const removeCompoundIndex = (indexName: string) => {
    updateNodeType({
      compound_indexes: (nodeType.compound_indexes || []).filter((idx) => idx.name !== indexName),
    })
  }

  const updateCompoundIndex = (indexName: string, updates: Partial<CompoundIndexDefinition>) => {
    updateNodeType({
      compound_indexes: (nodeType.compound_indexes || []).map((idx) =>
        idx.name === indexName ? { ...idx, ...updates } : idx
      ),
    })
  }

  const addColumnToIndex = (indexName: string) => {
    const index = (nodeType.compound_indexes || []).find((idx) => idx.name === indexName)
    if (!index) return

    const newColumn: CompoundIndexColumn = { property: '' }
    updateCompoundIndex(indexName, {
      columns: [...index.columns, newColumn],
    })
  }

  const removeColumnFromIndex = (indexName: string, columnIndex: number) => {
    const index = (nodeType.compound_indexes || []).find((idx) => idx.name === indexName)
    if (!index) return

    updateCompoundIndex(indexName, {
      columns: index.columns.filter((_, i) => i !== columnIndex),
    })
  }

  const updateColumn = (indexName: string, columnIndex: number, updates: Partial<CompoundIndexColumn>) => {
    const index = (nodeType.compound_indexes || []).find((idx) => idx.name === indexName)
    if (!index) return

    updateCompoundIndex(indexName, {
      columns: index.columns.map((col, i) => (i === columnIndex ? { ...col, ...updates } : col)),
    })
  }

  // Get available properties for compound index columns
  const getAvailableProperties = (): { value: string; label: string }[] => {
    const systemProps = [
      { value: '__node_type', label: 'node_type (system)' },
      { value: '__created_at', label: 'created_at (system)' },
      { value: '__updated_at', label: 'updated_at (system)' },
    ]

    const userProps = (nodeType.properties || [])
      .filter((p) => p.name && ['String', 'Number', 'Boolean', 'Date'].includes(p.type))
      .map((p) => ({ value: p.name!, label: p.name! }))

    return [...systemProps, ...userProps]
  }

  return (
    <div className="h-full flex flex-col bg-zinc-900/50 border-l border-white/10 backdrop-blur-sm overflow-hidden">
      <div className="px-3 py-2 border-b border-white/10">
        <div className="flex items-center gap-2">
          <Settings2 className="w-4 h-4 text-primary-400" />
          <h3 className="text-sm font-semibold text-white">Node Type Settings</h3>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {/* Name */}
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Name <span className="text-red-400">*</span>
          </label>
          <input
            type="text"
            value={nodeType.name}
            onChange={(e) => updateNodeType({ name: e.target.value })}
            className={`
              w-full px-3 py-2 bg-white/5 border rounded-lg text-sm text-white
              focus:outline-none focus:ring-2
              ${
                validationErrors.name
                  ? 'border-red-500/50 focus:ring-red-500/50'
                  : 'border-white/20 focus:ring-primary-500/50'
              }
            `}
            placeholder="namespace:TypeName"
          />
          {validationErrors.name && (
            <p className="text-xs text-red-400 mt-1">{validationErrors.name}</p>
          )}
          <p className="text-xs text-zinc-500 mt-1">
            Format: namespace:TypeName (e.g., raisin:Page)
          </p>
        </div>

        {/* Extends */}
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Extends (Parent Type)
          </label>
          <NodeTypePicker
            mode="single"
            value={nodeType.extends || ''}
            onChange={(value) => updateNodeType({ extends: (value as string) || undefined })}
            allowNone
            noneLabel="None (no parent)"
            excludeNames={nodeType.name ? [nodeType.name] : []}
          />
          {validationErrors.extends && (
            <p className="text-xs text-red-400 mt-1">{validationErrors.extends}</p>
          )}
        </div>

        {/* Mixins */}
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Mixins
          </label>
          <NodeTypePicker
            mode="multi"
            value={nodeType.mixins || []}
            onChange={(value) => updateNodeType({ mixins: (value as string[]).length > 0 ? (value as string[]) : undefined })}
            excludeNames={nodeType.name ? [nodeType.name] : []}
            placeholder="Select mixins to compose..."
          />
          <p className="text-xs text-zinc-500 mt-1">
            Reusable property sets to compose into this node type.
          </p>

          {/* Inherited Properties Preview */}
          {mixins.length > 0 && (
            <div className="mt-2 rounded-lg border border-white/10 bg-white/5 overflow-hidden">
              <button
                onClick={() => setMixinExpanded(!mixinExpanded)}
                className="w-full flex items-center gap-1.5 px-2.5 py-1.5 text-xs font-medium text-zinc-300 hover:bg-white/5 transition-colors"
              >
                {mixinExpanded ? (
                  <ChevronDown className="w-3 h-3 text-zinc-400" />
                ) : (
                  <ChevronRight className="w-3 h-3 text-zinc-400" />
                )}
                Inherited Properties
                {!mixinLoading && mixinProperties.length > 0 && (
                  <span className="text-zinc-500 font-normal">({mixinProperties.length})</span>
                )}
                {mixinLoading && <Loader2 className="w-3 h-3 text-zinc-500 animate-spin ml-auto" />}
              </button>

              {mixinExpanded && !mixinLoading && (
                <div className="border-t border-white/5">
                  {mixinProperties.length === 0 ? (
                    <p className="px-2.5 py-2 text-xs text-zinc-500 italic">
                      No properties defined in selected mixins
                    </p>
                  ) : (
                    <div className="divide-y divide-white/5">
                      {mixinProperties.map((prop) => (
                        <div key={`${prop.source}:${prop.name}`} className="px-2.5 py-1.5 flex items-baseline gap-1.5">
                          <span className="text-xs text-zinc-200">{prop.name}</span>
                          <span className="text-xs text-zinc-500">
                            ({prop.type}{prop.required ? ', required' : ''})
                          </span>
                          <span className="text-xs text-zinc-600 ml-auto truncate">
                            from {prop.source.includes(':') ? prop.source.split(':').pop() : prop.source}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Icon & Version */}
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-xs font-medium text-zinc-300 mb-1">
              Icon
            </label>
            <input
              type="text"
              value={nodeType.icon || ''}
              onChange={(e) => updateNodeType({ icon: e.target.value || undefined })}
              className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
              placeholder="icon-name"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-zinc-300 mb-1">
              Version
            </label>
            <input
              type="number"
              value={nodeType.version || ''}
              onChange={(e) =>
                updateNodeType({
                  version: e.target.value ? parseInt(e.target.value) : undefined,
                })
              }
              className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
              placeholder="1"
            />
          </div>
        </div>

        {/* Description */}
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Description
          </label>
          <textarea
            value={nodeType.description || ''}
            onChange={(e) => updateNodeType({ description: e.target.value || undefined })}
            rows={3}
            className="w-full px-3 py-2 bg-white/5 border border-white/20 rounded-lg text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50 resize-none"
            placeholder="Brief description of this node type..."
          />
        </div>

        {/* Flags */}
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-2">
            Settings
          </label>
          <div className="space-y-2">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={nodeType.strict || false}
                onChange={(e) => updateNodeType({ strict: e.target.checked })}
                className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
              />
              <span className="text-sm text-zinc-300">Strict mode</span>
            </label>

            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={nodeType.versionable !== false}
                onChange={(e) => updateNodeType({ versionable: e.target.checked })}
                className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
              />
              <span className="text-sm text-zinc-300">Versionable</span>
            </label>

            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={nodeType.publishable !== false}
                onChange={(e) => updateNodeType({ publishable: e.target.checked })}
                className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
              />
              <span className="text-sm text-zinc-300">Publishable</span>
            </label>

            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={nodeType.auditable !== false}
                onChange={(e) => updateNodeType({ auditable: e.target.checked })}
                className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
              />
              <span className="text-sm text-zinc-300">Auditable</span>
            </label>
          </div>
        </div>

        {/* Indexing Settings */}
        <div className="space-y-3 p-3 bg-white/5 rounded-lg border border-white/10">
          <label className="block text-xs font-medium text-zinc-300">Indexing</label>

          {/* Master Toggle */}
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={nodeType.indexable !== false}
              onChange={(e) => updateNodeType({ indexable: e.target.checked })}
              className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50"
            />
            <span className="text-sm text-zinc-300">Enable indexing</span>
          </label>

          <p className="text-xs text-zinc-500">
            When disabled, this node type will not be indexed in any index (fulltext, vector, or
            property).
          </p>

          {/* Index Types Selection */}
          {nodeType.indexable !== false && (
            <div className="space-y-2 pl-6 border-l-2 border-primary-500/30">
              <label className="block text-xs font-medium text-zinc-400">
                Available index types
              </label>

              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={nodeType.index_types ? nodeType.index_types.includes('Fulltext') : true}
                  onChange={(e) => updateIndexTypes(e.target.checked, 'Fulltext')}
                  className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
                />
                <div className="flex-1">
                  <span className="text-sm text-zinc-300 block">Fulltext Search</span>
                  <p className="text-xs text-zinc-500 mt-0.5">
                    Tantivy-based full-text search with stemming and language support
                  </p>
                </div>
              </label>

              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={nodeType.index_types ? nodeType.index_types.includes('Vector') : true}
                  onChange={(e) => updateIndexTypes(e.target.checked, 'Vector')}
                  className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
                />
                <div className="flex-1">
                  <span className="text-sm text-zinc-300 block">Vector Embeddings</span>
                  <p className="text-xs text-zinc-500 mt-0.5">
                    AI-powered semantic search using OpenAI embeddings (requires tenant config)
                  </p>
                </div>
              </label>

              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={nodeType.index_types ? nodeType.index_types.includes('Property') : true}
                  onChange={(e) => updateIndexTypes(e.target.checked, 'Property')}
                  className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
                />
                <div className="flex-1">
                  <span className="text-sm text-zinc-300 block">Property Index</span>
                  <p className="text-xs text-zinc-500 mt-0.5">
                    Fast exact-match lookups on property values (stored in RocksDB)
                  </p>
                </div>
              </label>
            </div>
          )}
        </div>

        {/* Compound Indexes */}
        <div className="space-y-3 p-3 bg-white/5 rounded-lg border border-white/10">
          <div className="flex items-center justify-between">
            <label className="block text-xs font-medium text-zinc-300">Compound Indexes</label>
            <button
              onClick={addCompoundIndex}
              className="flex items-center gap-1 px-2 py-1 text-xs text-primary-400 hover:text-primary-300 hover:bg-white/5 rounded transition-colors"
            >
              <Plus className="w-3 h-3" />
              Add Index
            </button>
          </div>

          <p className="text-xs text-zinc-500">
            Compound indexes optimize queries with multiple WHERE conditions + ORDER BY. E.g.,
            WHERE node_type = 'Article' AND category = 'news' ORDER BY created_at DESC LIMIT 10.
          </p>

          {(nodeType.compound_indexes || []).length === 0 ? (
            <p className="text-xs text-zinc-500 italic py-2">No compound indexes defined</p>
          ) : (
            <div className="space-y-3">
              {(nodeType.compound_indexes || []).map((index) => (
                <div
                  key={index.name}
                  className="p-2 bg-black/20 rounded-lg border border-white/5 space-y-2"
                >
                  {/* Index Header */}
                  <div className="flex items-center justify-between">
                    <input
                      type="text"
                      value={index.name}
                      onChange={(e) => updateCompoundIndex(index.name, { name: e.target.value })}
                      className="flex-1 px-2 py-1 bg-white/5 border border-white/10 rounded text-xs text-white focus:outline-none focus:ring-1 focus:ring-primary-500/50"
                      placeholder="Index name"
                    />
                    <button
                      onClick={() => removeCompoundIndex(index.name)}
                      className="ml-2 p-1 text-red-400 hover:text-red-300 hover:bg-red-500/10 rounded transition-colors"
                    >
                      <Trash2 className="w-3 h-3" />
                    </button>
                  </div>

                  {/* Index Columns */}
                  <div className="space-y-1">
                    <div className="flex items-center justify-between">
                      <span className="text-xs text-zinc-400">Columns (in order)</span>
                      <button
                        onClick={() => addColumnToIndex(index.name)}
                        className="text-xs text-primary-400 hover:text-primary-300"
                      >
                        + Add Column
                      </button>
                    </div>

                    {index.columns.map((col, colIndex) => (
                      <div key={colIndex} className="flex items-center gap-2">
                        <GripVertical className="w-3 h-3 text-zinc-500" />
                        <select
                          value={col.property}
                          onChange={(e) =>
                            updateColumn(index.name, colIndex, { property: e.target.value })
                          }
                          className="flex-1 px-2 py-1 bg-white/5 border border-white/10 rounded text-xs text-white focus:outline-none focus:ring-1 focus:ring-primary-500/50"
                        >
                          <option value="">Select property...</option>
                          {getAvailableProperties().map((prop) => (
                            <option key={prop.value} value={prop.value}>
                              {prop.label}
                            </option>
                          ))}
                        </select>

                        {/* Show sort direction for last column if has_order_column */}
                        {index.has_order_column && colIndex === index.columns.length - 1 && (
                          <select
                            value={col.ascending ? 'asc' : 'desc'}
                            onChange={(e) =>
                              updateColumn(index.name, colIndex, {
                                ascending: e.target.value === 'asc',
                              })
                            }
                            className="w-16 px-1 py-1 bg-white/5 border border-white/10 rounded text-xs text-white focus:outline-none focus:ring-1 focus:ring-primary-500/50"
                          >
                            <option value="desc">DESC</option>
                            <option value="asc">ASC</option>
                          </select>
                        )}

                        <button
                          onClick={() => removeColumnFromIndex(index.name, colIndex)}
                          disabled={index.columns.length <= 1}
                          className="p-1 text-zinc-500 hover:text-red-400 disabled:opacity-30 disabled:cursor-not-allowed rounded transition-colors"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    ))}
                  </div>

                  {/* Has Order Column Toggle */}
                  <label className="flex items-center gap-2 cursor-pointer pt-1 border-t border-white/5">
                    <input
                      type="checkbox"
                      checked={index.has_order_column}
                      onChange={(e) =>
                        updateCompoundIndex(index.name, { has_order_column: e.target.checked })
                      }
                      className="w-3 h-3 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-1 focus:ring-primary-500/50"
                    />
                    <span className="text-xs text-zinc-400">
                      Last column is for ORDER BY (timestamp)
                    </span>
                  </label>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Allowed Children */}
        <div>
          <label className="block text-xs font-medium text-zinc-300 mb-1">
            Allowed Children
          </label>
          <NodeTypePicker
            mode="multi"
            value={nodeType.allowed_children}
            onChange={(value) => updateNodeType({ allowed_children: value as string[] })}
            allowWildcard
            placeholder="Select allowed child types..."
          />
          <p className="text-xs text-zinc-500 mt-1">
            Select types that can be children of this node type. Use "Allow All" for any type.
          </p>
        </div>
      </div>
    </div>
  )
}
