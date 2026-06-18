/**
 * Field Indexing Panel
 *
 * Per-field index-type picker for the Archetype / ElementType visual field
 * editors. Mirrors the NodeType index-type picker
 * (nodetype-builder/CoreSettingsPanel.tsx) — same labels, descriptions and
 * Tailwind styling — but operates on a single field's `index?: IndexType[]`.
 *
 * Semantics: a field is identity-only (NOT indexed) by default. Checking a box
 * adds that index type to the array; unchecking removes it; when none remain the
 * value is set back to `undefined` so the field is not indexed at all.
 */

import type { IndexType } from '../archetype-builder/types'

interface FieldIndexingPanelProps {
  value?: IndexType[]
  onChange: (next: IndexType[] | undefined) => void
}

interface IndexTypeOption {
  type: IndexType
  label: string
  description: string
}

const INDEX_TYPE_OPTIONS: IndexTypeOption[] = [
  {
    type: 'Fulltext',
    label: 'Fulltext Search',
    description: 'Tantivy-based full-text search with stemming and language support',
  },
  {
    type: 'Vector',
    label: 'Vector Embeddings',
    description: 'AI-powered semantic search using OpenAI embeddings (requires tenant config)',
  },
  {
    type: 'Property',
    label: 'Property Index',
    description: 'Fast exact-match lookups on property values (stored in RocksDB)',
  },
]

export default function FieldIndexingPanel({ value, onChange }: FieldIndexingPanelProps) {
  const toggleIndexType = (enabled: boolean, indexType: IndexType) => {
    const current = value || []

    const updated = enabled
      ? [...current, indexType].filter((v, i, a) => a.indexOf(v) === i) // Add and dedupe
      : current.filter((t) => t !== indexType) // Remove

    // No types selected → not indexed (identity-only default)
    onChange(updated.length > 0 ? updated : undefined)
  }

  return (
    <div className="space-y-3 p-3 bg-white/5 rounded-lg border border-white/10">
      <label className="block text-xs font-medium text-zinc-300">Field indexing</label>

      <p className="text-xs text-zinc-500">
        Select which indexes include this field's value. Unchecked = not indexed (the
        element/archetype identity is always searchable).
      </p>

      <div className="space-y-2 pl-6 border-l-2 border-primary-500/30">
        {INDEX_TYPE_OPTIONS.map((option) => (
          <label key={option.type} className="flex items-start gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={value ? value.includes(option.type) : false}
              onChange={(e) => toggleIndexType(e.target.checked, option.type)}
              className="w-4 h-4 rounded border-white/20 bg-white/5 text-primary-500 focus:ring-2 focus:ring-primary-500/50 mt-0.5"
            />
            <div className="flex-1">
              <span className="text-sm text-zinc-300 block">{option.label}</span>
              <p className="text-xs text-zinc-500 mt-0.5">{option.description}</p>
            </div>
          </label>
        ))}
      </div>
    </div>
  )
}
