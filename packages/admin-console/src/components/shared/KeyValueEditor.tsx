/**
 * KeyValueEditor
 *
 * Reusable editor for free-form key/value maps such as a schema's `meta`
 * or a property's `constraints`. Renders a list of key + value rows.
 *
 * Values are parsed as JSON when possible (so `5` becomes a number, `true`
 * a boolean, `["a","b"]` an array, `{...}` an object); anything that is not
 * valid JSON is kept verbatim as a string. This mirrors how the backend
 * stores these maps (`HashMap<String, PropertyValue>`), which accepts
 * arbitrary scalar, array, or object values.
 */

import { useEffect, useRef, useState } from 'react'
import { Plus, X } from 'lucide-react'
import { nanoid } from 'nanoid'

interface Row {
  id: string
  k: string
  v: string
}

function toRows(obj?: Record<string, any>): Row[] {
  if (!obj || typeof obj !== 'object') return []
  return Object.entries(obj).map(([k, val]) => ({
    id: nanoid(),
    k,
    v: typeof val === 'string' ? val : JSON.stringify(val),
  }))
}

/** Parse a raw text value into the richest type it represents. */
export function parseKeyValue(raw: string): any {
  const trimmed = raw.trim()
  if (trimmed === '') return ''
  try {
    return JSON.parse(trimmed)
  } catch {
    return raw
  }
}

interface KeyValueEditorProps {
  value?: Record<string, any>
  onChange: (value: Record<string, any> | undefined) => void
  /**
   * Reseed the internal rows when this changes. Pass the id of the entity
   * being edited (e.g. the selected field or property) so switching the
   * selection refreshes the rows without clobbering in-progress typing.
   */
  instanceKey?: string
  keyPlaceholder?: string
  valuePlaceholder?: string
  addLabel?: string
}

export default function KeyValueEditor({
  value,
  onChange,
  instanceKey,
  keyPlaceholder = 'key',
  valuePlaceholder = 'value (text or JSON)',
  addLabel = 'Add entry',
}: KeyValueEditorProps) {
  const [rows, setRows] = useState<Row[]>(() => toRows(value))
  const prevKey = useRef(instanceKey)

  // Reseed only when the edited entity changes, so typing an empty key or
  // value is never wiped out by the parent re-rendering after onChange.
  useEffect(() => {
    if (prevKey.current !== instanceKey) {
      prevKey.current = instanceKey
      setRows(toRows(value))
    }
  }, [instanceKey, value])

  const emit = (next: Row[]) => {
    const obj: Record<string, any> = {}
    next.forEach(({ k, v }) => {
      const key = k.trim()
      if (!key) return
      obj[key] = parseKeyValue(v)
    })
    onChange(Object.keys(obj).length > 0 ? obj : undefined)
  }

  const setAndEmit = (next: Row[]) => {
    setRows(next)
    emit(next)
  }

  const addRow = () => setRows([...rows, { id: nanoid(), k: '', v: '' }])
  const updateRow = (id: string, patch: Partial<Row>) =>
    setAndEmit(rows.map((r) => (r.id === id ? { ...r, ...patch } : r)))
  const removeRow = (id: string) => setAndEmit(rows.filter((r) => r.id !== id))

  return (
    <div className="space-y-2">
      {rows.length > 0 && (
        <div className="space-y-2">
          {rows.map((row) => (
            <div key={row.id} className="flex items-center gap-1">
              <input
                type="text"
                value={row.k}
                onChange={(e) => updateRow(row.id, { k: e.target.value })}
                className="flex-1 min-w-0 px-2 py-1.5 bg-white/5 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                placeholder={keyPlaceholder}
              />
              <input
                type="text"
                value={row.v}
                onChange={(e) => updateRow(row.id, { v: e.target.value })}
                className="flex-1 min-w-0 px-2 py-1.5 bg-white/5 border border-white/20 rounded text-sm text-white font-mono focus:outline-none focus:ring-2 focus:ring-primary-500/50"
                placeholder={valuePlaceholder}
              />
              <button
                type="button"
                onClick={() => removeRow(row.id)}
                className="p-1 hover:bg-red-500/20 text-red-400 rounded transition-colors flex-shrink-0"
                title="Remove entry"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}

      <button
        type="button"
        onClick={addRow}
        className="flex items-center gap-1 px-2 py-1 text-xs bg-primary-500/20 hover:bg-primary-500/30 text-primary-300 rounded transition-colors"
      >
        <Plus className="w-3 h-3" />
        {addLabel}
      </button>
    </div>
  )
}
