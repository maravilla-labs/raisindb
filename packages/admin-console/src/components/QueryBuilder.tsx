import { useState } from 'react'
import { X, Plus, Filter, Play, ChevronDown, ChevronUp } from 'lucide-react'

export interface QueryFilter {
  id: string
  field: string
  operator: string
  value: string | string[]
}

export interface QueryBuilderProps {
  onExecute: (query: any) => void
  onClose: () => void
}

const FIELD_OPTIONS = [
  { value: 'id', label: 'ID' },
  { value: 'name', label: 'Name' },
  { value: 'path', label: 'Path' },
  { value: 'node_type', label: 'Node Type' },
  { value: 'parent', label: 'Parent' },
]

const OPERATOR_OPTIONS = [
  { value: 'eq', label: 'Equals' },
  { value: 'ne', label: 'Not Equals' },
  { value: 'like', label: 'Contains' },
  { value: 'in', label: 'In List' },
  { value: 'exists', label: 'Exists' },
  { value: 'gt', label: 'Greater Than' },
  { value: 'lt', label: 'Less Than' },
  { value: 'gte', label: 'Greater or Equal' },
  { value: 'lte', label: 'Less or Equal' },
]

const LOGIC_OPTIONS = [
  { value: 'and', label: 'AND (all must match)' },
  { value: 'or', label: 'OR (any can match)' },
]

const SORT_OPTIONS = [
  { value: 'path', label: 'Path' },
  { value: 'id', label: 'ID' },
  { value: 'name', label: 'Name' },
  { value: 'node_type', label: 'Node Type' },
]

export default function QueryBuilder({ onExecute, onClose }: QueryBuilderProps) {
  const [filters, setFilters] = useState<QueryFilter[]>([
    { id: crypto.randomUUID(), field: 'name', operator: 'eq', value: '' },
  ])
  const [logic, setLogic] = useState<'and' | 'or'>('and')
  const [negated, setNegated] = useState(false)
  const [sortField, setSortField] = useState<string>('')
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc'>('asc')
  const [limit, setLimit] = useState<string>('100')
  const [offset, setOffset] = useState<string>('0')
  const [showJson, setShowJson] = useState(false)

  function addFilter() {
    setFilters([
      ...filters,
      { id: crypto.randomUUID(), field: 'name', operator: 'eq', value: '' },
    ])
  }

  function removeFilter(id: string) {
    setFilters(filters.filter(f => f.id !== id))
  }

  function updateFilter(id: string, field: keyof QueryFilter, value: any) {
    setFilters(filters.map(f => (f.id === id ? { ...f, [field]: value } : f)))
  }

  function buildQuery(): any {
    const fieldFilters = filters
      .filter(f => f.value !== '' && f.value.length > 0)
      .map(f => {
        const ops: any = {}

        // Handle special operators
        if (f.operator === 'exists') {
          const strValue = typeof f.value === 'string' ? f.value : String(f.value)
          ops[f.operator] = strValue === 'true'
        } else if (f.operator === 'in') {
          // Split comma-separated values
          const values = typeof f.value === 'string'
            ? f.value.split(',').map(v => v.trim())
            : f.value
          ops[f.operator] = values
        } else {
          ops[f.operator] = f.value
        }

        return { [f.field]: ops }
      })

    if (fieldFilters.length === 0) {
      return {}
    }

    const filtersList = fieldFilters.map(ff => ({ ...ff }))

    let query: any = {}

    // Apply logic (and/or)
    if (negated) {
      query.not = { [logic]: filtersList }
    } else {
      query[logic] = filtersList
    }

    // Add sorting
    if (sortField) {
      query.order_by = { [sortField]: sortOrder }
    }

    // Add pagination
    const limitNum = parseInt(limit) || 100
    const offsetNum = parseInt(offset) || 0
    if (limitNum > 0) {
      query.limit = limitNum
    }
    if (offsetNum > 0) {
      query.offset = offsetNum
    }

    return query
  }

  function handleExecute() {
    const query = buildQuery()
    onExecute(query)
  }

  const query = buildQuery()

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl shadow-2xl max-w-4xl w-full max-h-[90vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-white/10">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-primary-500/20 rounded-lg">
              <Filter className="w-5 h-5 text-primary-400" />
            </div>
            <div>
              <h2 className="text-xl font-semibold text-white">Query Builder</h2>
              <p className="text-sm text-gray-400">Build and execute custom queries</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors text-gray-400 hover:text-white"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {/* Logic Selection */}
          <div className="space-y-2">
            <label className="block text-sm font-medium text-gray-300">Query Logic</label>
            <div className="flex gap-3">
              {LOGIC_OPTIONS.map(option => (
                <button
                  key={option.value}
                  onClick={() => setLogic(option.value as 'and' | 'or')}
                  className={`flex-1 px-4 py-2 rounded-lg border transition-colors ${
                    logic === option.value
                      ? 'bg-primary-500/20 border-primary-400/50 text-primary-300'
                      : 'bg-white/5 border-white/10 text-gray-400 hover:bg-white/10'
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>

            {/* Negation Toggle */}
            <div className="flex items-center gap-2 pt-2">
              <input
                type="checkbox"
                id="negate"
                checked={negated}
                onChange={(e) => setNegated(e.target.checked)}
                className="rounded border-white/20 bg-white/10 text-primary-500 focus:ring-primary-500"
              />
              <label htmlFor="negate" className="text-sm text-gray-300">
                Negate query (NOT)
              </label>
            </div>
          </div>

          {/* Filters */}
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <label className="block text-sm font-medium text-gray-300">Filters</label>
              <button
                onClick={addFilter}
                className="flex items-center gap-2 px-3 py-1.5 bg-primary-500/20 border border-primary-400/30 rounded-lg text-primary-300 text-sm hover:bg-primary-500/30 transition-colors"
              >
                <Plus className="w-4 h-4" />
                Add Filter
              </button>
            </div>

            {filters.map((filter) => (
              <div
                key={filter.id}
                className="flex gap-2 items-start p-4 bg-white/5 border border-white/10 rounded-lg"
              >
                <div className="flex-1 grid grid-cols-1 md:grid-cols-3 gap-2">
                  {/* Field */}
                  <select
                    value={filter.field}
                    onChange={(e) => updateFilter(filter.id, 'field', e.target.value)}
                    className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                  >
                    {FIELD_OPTIONS.map(opt => (
                      <option key={opt.value} value={opt.value}>{opt.label}</option>
                    ))}
                  </select>

                  {/* Operator */}
                  <select
                    value={filter.operator}
                    onChange={(e) => updateFilter(filter.id, 'operator', e.target.value)}
                    className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                  >
                    {OPERATOR_OPTIONS.map(opt => (
                      <option key={opt.value} value={opt.value}>{opt.label}</option>
                    ))}
                  </select>

                  {/* Value */}
                  {filter.operator === 'exists' ? (
                    <select
                      value={filter.value as string}
                      onChange={(e) => updateFilter(filter.id, 'value', e.target.value)}
                      className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                    >
                      <option value="true">True</option>
                      <option value="false">False</option>
                    </select>
                  ) : filter.operator === 'in' ? (
                    <input
                      type="text"
                      value={filter.value as string}
                      onChange={(e) => updateFilter(filter.id, 'value', e.target.value)}
                      placeholder="value1, value2, value3"
                      className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
                    />
                  ) : (
                    <input
                      type="text"
                      value={filter.value as string}
                      onChange={(e) => updateFilter(filter.id, 'value', e.target.value)}
                      placeholder="Enter value"
                      className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-primary-500"
                    />
                  )}
                </div>

                {/* Remove button */}
                {filters.length > 1 && (
                  <button
                    onClick={() => removeFilter(filter.id)}
                    className="p-2 hover:bg-red-500/20 rounded-lg transition-colors text-gray-400 hover:text-red-400"
                  >
                    <X className="w-4 h-4" />
                  </button>
                )}
              </div>
            ))}
          </div>

          {/* Sorting */}
          <div className="space-y-2">
            <label className="block text-sm font-medium text-gray-300">Sorting</label>
            <div className="grid grid-cols-2 gap-2">
              <select
                value={sortField}
                onChange={(e) => setSortField(e.target.value)}
                className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
              >
                <option value="">No sorting</option>
                {SORT_OPTIONS.map(opt => (
                  <option key={opt.value} value={opt.value}>{opt.label}</option>
                ))}
              </select>
              <select
                value={sortOrder}
                onChange={(e) => setSortOrder(e.target.value as 'asc' | 'desc')}
                disabled={!sortField}
                className="px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500 disabled:opacity-50"
              >
                <option value="asc">Ascending</option>
                <option value="desc">Descending</option>
              </select>
            </div>
          </div>

          {/* Pagination */}
          <div className="space-y-2">
            <label className="block text-sm font-medium text-gray-300">Pagination</label>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="block text-xs text-gray-400 mb-1">Limit</label>
                <input
                  type="number"
                  value={limit}
                  onChange={(e) => setLimit(e.target.value)}
                  min="1"
                  className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                />
              </div>
              <div>
                <label className="block text-xs text-gray-400 mb-1">Offset</label>
                <input
                  type="number"
                  value={offset}
                  onChange={(e) => setOffset(e.target.value)}
                  min="0"
                  className="w-full px-3 py-2 bg-black/30 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                />
              </div>
            </div>
          </div>

          {/* JSON Preview */}
          <div className="space-y-2">
            <button
              onClick={() => setShowJson(!showJson)}
              className="flex items-center gap-2 text-sm font-medium text-gray-300 hover:text-white transition-colors"
            >
              {showJson ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
              Query JSON Preview
            </button>

            {showJson && (
              <pre className="p-4 bg-black/50 border border-white/10 rounded-lg text-sm text-gray-300 overflow-x-auto">
                {JSON.stringify(query, null, 2)}
              </pre>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 p-6 border-t border-white/10">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleExecute}
            className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Play className="w-4 h-4" />
            Execute Query
          </button>
        </div>
      </div>
    </div>
  )
}
