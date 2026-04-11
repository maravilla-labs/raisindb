import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { Layout, Plus, Trash2, GripVertical, ChevronDown, ChevronRight } from 'lucide-react'
import { draggable, dropTargetForElements, monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'
import { attachClosestEdge, extractClosestEdge, type Edge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { reorderWithEdge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/util/reorder-with-edge'
import ArchetypeFieldRenderer from '../ArchetypeFieldRenderer'
import type { FieldSchema } from '../../api/archetypes'

interface CompositeFieldEditorProps {
  name: string
  label: string
  value: Record<string, unknown> | Record<string, unknown>[] | undefined
  error?: string
  onChange: (value: unknown) => void
  /** Inline sub-field definitions from the CompositeField schema */
  fields?: FieldSchema[]
  repo?: string
  branch?: string
  translationMode?: boolean
  /** Original (source-language) composite value for translation reference */
  originalValue?: unknown
  /** Default language code for "Original (en):" hints */
  defaultLanguage?: string
  /** Whether this composite field supports multiple items (repeatable) */
  multiple?: boolean
}

/** Drop indicator line with circles at edges */
function ListDropIndicator({ edge }: { edge: Edge }) {
  const isTop = edge === 'top'
  return (
    <div
      className="absolute left-0 right-0 flex items-center pointer-events-none z-10"
      style={{
        top: isTop ? -1 : undefined,
        bottom: isTop ? undefined : -1,
        transform: isTop ? 'translateY(-50%)' : 'translateY(50%)',
      }}
    >
      <div className="w-2 h-2 rounded-full bg-primary-500 flex-shrink-0" />
      <div className="flex-1 h-0.5 bg-primary-500" />
      <div className="w-2 h-2 rounded-full bg-primary-500 flex-shrink-0" />
    </div>
  )
}

/** Renders sub-fields for a single composite object */
function CompositeItemFields({
  item,
  fields,
  originalItem,
  defaultLanguage,
  onFieldChange,
  repo,
  branch,
  translationMode,
}: {
  item: Record<string, unknown>
  fields: FieldSchema[]
  originalItem?: Record<string, unknown>
  defaultLanguage?: string
  onFieldChange: (fieldName: string, fieldValue: unknown) => void
  repo?: string
  branch?: string
  translationMode?: boolean
}) {
  return (
    <>
      {fields.map((fieldSchema: any) => {
        const fieldName = fieldSchema.base?.name ?? fieldSchema.name
        if (!fieldName) return null
        return (
          <ArchetypeFieldRenderer
            key={fieldName}
            field={fieldSchema}
            value={item[fieldName]}
            onChange={(v) => onFieldChange(fieldName, v)}
            translationMode={translationMode}
            originalValue={originalItem?.[fieldName]}
            defaultLanguage={defaultLanguage}
            repo={repo}
            branch={branch}
          />
        )
      })}
    </>
  )
}

/** Renders a single item card within an array composite */
function CompositeItemCard({
  item,
  index,
  fields,
  fieldName,
  originalItem,
  defaultLanguage,
  onUpdate,
  onRemove,
  repo,
  branch,
  translationMode,
}: {
  item: Record<string, unknown>
  index: number
  fields: FieldSchema[]
  fieldName: string
  originalItem?: Record<string, unknown>
  defaultLanguage?: string
  onUpdate: (index: number, updated: Record<string, unknown>) => void
  onRemove: (index: number) => void
  repo?: string
  branch?: string
  translationMode?: boolean
}) {
  const [expanded, setExpanded] = useState(true)
  const [isDragging, setIsDragging] = useState(false)
  const [closestEdge, setClosestEdge] = useState<Edge | null>(null)
  const cardRef = useRef<HTMLDivElement>(null)
  const handleRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = cardRef.current
    const handle = handleRef.current
    if (!el || !handle || translationMode) return

    const cleanup = combine(
      draggable({
        element: el,
        dragHandle: handle,
        getInitialData: () => ({
          type: 'composite-item',
          fieldName,
          index,
        }),
        onDragStart: () => setIsDragging(true),
        onDrop: () => setIsDragging(false),
      }),
      dropTargetForElements({
        element: el,
        canDrop: ({ source }) => {
          const data = source.data
          return (
            data.type === 'composite-item' &&
            data.fieldName === fieldName &&
            data.index !== index
          )
        },
        getData: ({ input, element }) =>
          attachClosestEdge(
            { type: 'composite-item', fieldName, index },
            { element, input, allowedEdges: ['top', 'bottom'] },
          ),
        onDrag: ({ self }) => {
          setClosestEdge(extractClosestEdge(self.data))
        },
        onDragLeave: () => setClosestEdge(null),
        onDrop: () => setClosestEdge(null),
      }),
    )

    return cleanup
  }, [index, fieldName, translationMode])

  // Build a short summary from the item's values for the collapsed header
  const summary =
    (item.title as string) ??
    (item.name as string) ??
    (item.label as string) ??
    `Item ${index + 1}`

  return (
    <div
      ref={cardRef}
      className="relative bg-white/5 border border-white/10 rounded-lg"
      style={{ opacity: isDragging ? 0.4 : 1 }}
    >
      {closestEdge && <ListDropIndicator edge={closestEdge} />}
      <div className="flex items-center gap-2 px-3 py-2">
        {!translationMode && (
          <div ref={handleRef} className="flex-shrink-0 cursor-grab">
            <GripVertical className="w-4 h-4 text-zinc-500" />
          </div>
        )}
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 text-sm text-zinc-300 hover:text-white flex-1 min-w-0"
        >
          {expanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
          <span className="font-medium truncate">{summary}</span>
        </button>
        {!translationMode && (
          <button
            type="button"
            onClick={() => onRemove(index)}
            className="p-1 text-red-400 hover:text-red-300 hover:bg-red-500/20 rounded"
            title="Remove item"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        )}
      </div>
      {expanded && (
        <div className="px-4 pb-4 space-y-3 border-t border-white/5 pt-3">
          <CompositeItemFields
            item={item}
            fields={fields}
            originalItem={originalItem}
            defaultLanguage={defaultLanguage}
            onFieldChange={(fn, fv) =>
              onUpdate(index, { ...item, [fn]: fv })
            }
            repo={repo}
            branch={branch}
            translationMode={translationMode}
          />
        </div>
      )}
    </div>
  )
}

export default function CompositeFieldEditor({
  name: _name,
  label,
  value,
  error,
  onChange,
  fields,
  repo,
  branch,
  translationMode,
  originalValue,
  defaultLanguage,
  multiple,
}: CompositeFieldEditorProps) {
  // Use array mode if:
  // 1. The schema says multiple: true (repeatable composite)
  // 2. The current value is already an array
  // 3. In translation mode, the original value is an array
  const isArray =
    multiple || Array.isArray(value) || (!!translationMode && Array.isArray(originalValue))

  // --- Array mode: list of composite items ---
  if (isArray) {
    const items: Record<string, unknown>[] = Array.isArray(value) ? value : []
    const originalItems = Array.isArray(originalValue)
      ? (originalValue as Record<string, unknown>[])
      : undefined

    // In translation mode the ORIGINAL determines how many items exist and
    // their order. Translated values are overlaid by UUID (or index fallback).
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const displayItems: Record<string, unknown>[] = useMemo(() => {
      if (!translationMode || !originalItems) return items

      return originalItems.map((origItem, idx) => {
        const uuid = origItem.uuid as string | undefined
        const translated = uuid
          ? items.find((t) => t.uuid === uuid)
          : items[idx]
        if (!translated) return origItem
        return { ...origItem, ...translated }
      })
    }, [translationMode, originalItems, items])

    const handleUpdate = useCallback(
      (index: number, updated: Record<string, unknown>) => {
        // Extend items array if needed (original may have more items than
        // the current translation value)
        const next = [...items]
        while (next.length <= index) {
          // Gap-fill with UUID so server-side validation passes
          next.push({ uuid: crypto.randomUUID() })
        }
        next[index] = updated
        onChange(next)
      },
      [items, onChange],
    )

    const handleRemove = useCallback(
      (index: number) => {
        onChange(items.filter((_, i) => i !== index))
      },
      [items, onChange],
    )

    function handleAdd() {
      onChange([...items, { uuid: crypto.randomUUID() }])
    }

    // Monitor drops for this field's composite items
    // eslint-disable-next-line react-hooks/rules-of-hooks
    useEffect(() => {
      if (translationMode) return

      const cleanup = monitorForElements({
        canMonitor: ({ source }) => {
          const data = source.data
          return data.type === 'composite-item' && data.fieldName === _name
        },
        onDrop: ({ source, location }) => {
          const target = location.current.dropTargets[0]
          if (!target) return

          const sourceIndex = source.data.index as number
          const targetIndex = target.data.index as number
          const edge = extractClosestEdge(target.data)

          const reordered = reorderWithEdge({
            list: items,
            startIndex: sourceIndex,
            indexOfTarget: targetIndex,
            closestEdgeOfTarget: edge,
            axis: 'vertical',
          })

          onChange(reordered)
        },
      })

      return cleanup
    }, [items, onChange, _name, translationMode])

    return (
      <div>
        <label className="flex items-center gap-2 text-sm font-medium text-zinc-300 mb-3">
          <Layout className="w-4 h-4 text-purple-400" />
          {label}
        </label>

        <div className="space-y-2">
          {displayItems.map((item, idx) => (
            <CompositeItemCard
              key={idx}
              item={item}
              index={idx}
              fields={fields ?? []}
              fieldName={_name}
              originalItem={originalItems?.[idx]}
              defaultLanguage={defaultLanguage}
              onUpdate={handleUpdate}
              onRemove={handleRemove}
              repo={repo}
              branch={branch}
              translationMode={translationMode}
            />
          ))}
        </div>

        {displayItems.length === 0 && (
          <p className="text-sm text-zinc-500 py-2">No items yet</p>
        )}

        {!translationMode && (
          <button
            type="button"
            onClick={handleAdd}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-purple-500/20 hover:bg-purple-500/30 text-purple-300 rounded text-sm transition-colors mt-2"
          >
            <Plus className="w-3.5 h-3.5" />
            Add Item
          </button>
        )}

        {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
      </div>
    )
  }

  // --- Single object mode (original behavior) ---
  const current = (value as Record<string, unknown>) ?? {}
  const originalObj = (originalValue as Record<string, unknown>) ?? undefined

  function handleFieldChange(fieldName: string, fieldValue: unknown) {
    onChange({ ...current, [fieldName]: fieldValue })
  }

  return (
    <div>
      <label className="flex items-center gap-2 text-sm font-medium text-zinc-300 mb-3">
        <Layout className="w-4 h-4 text-purple-400" />
        {label}
      </label>

      <div className="bg-white/5 border border-white/10 rounded-lg p-4 space-y-3">
        {fields && fields.length > 0 ? (
          <CompositeItemFields
            item={current}
            fields={fields}
            originalItem={originalObj}
            defaultLanguage={defaultLanguage}
            onFieldChange={handleFieldChange}
            repo={repo}
            branch={branch}
            translationMode={translationMode}
          />
        ) : (
          // Fallback: render existing keys as plain inputs
          Object.keys(current).length > 0 ? (
            Object.entries(current).map(([key, val]) => (
              <div key={key}>
                <label className="block text-sm font-medium text-zinc-400 mb-1">{key}</label>
                <input
                  type="text"
                  value={typeof val === 'object' ? JSON.stringify(val) : String(val ?? '')}
                  onChange={(e) => {
                    try {
                      handleFieldChange(key, JSON.parse(e.target.value))
                    } catch {
                      handleFieldChange(key, e.target.value)
                    }
                  }}
                  className="w-full px-3 py-1.5 bg-white/10 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-1 focus:ring-primary-500"
                />
              </div>
            ))
          ) : (
            <p className="text-sm text-zinc-500">No sub-fields defined</p>
          )
        )}
      </div>

      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}
