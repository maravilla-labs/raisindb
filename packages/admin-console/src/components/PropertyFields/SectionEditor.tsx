import { useState, useMemo, useEffect, useRef, useCallback } from 'react'
import { Plus, Trash2, GripVertical, ChevronDown, ChevronRight, Layers } from 'lucide-react'
import { draggable, dropTargetForElements, monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'
import { attachClosestEdge, extractClosestEdge, type Edge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { reorderWithEdge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/util/reorder-with-edge'
import { useResolvedElementType } from '../../hooks/useResolvedElementType'
import ArchetypeFieldRenderer from '../ArchetypeFieldRenderer'

interface Element {
  element_type: string
  uuid: string
  [key: string]: unknown
}

type SectionValue = Element[] | { uuid?: string; items?: Element[] }

interface SectionEditorProps {
  name: string
  label: string
  value: SectionValue | undefined
  error?: string
  onChange: (value: unknown) => void
  allowedElementTypes?: string[]
  repo?: string
  branch?: string
  translationMode?: boolean
  /** Original (source-language) section value for translation reference */
  originalValue?: unknown
  /** Default language code for "Original (en):" hints */
  defaultLanguage?: string
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

/** Renders one element card within a section */
function ElementCard({
  element,
  index,
  sectionName,
  originalElement,
  defaultLanguage,
  onUpdate,
  onRemove,
  repo,
  branch,
  translationMode,
}: {
  element: Element
  index: number
  sectionName: string
  originalElement?: Element
  defaultLanguage?: string
  onUpdate: (index: number, updated: Element) => void
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

  const { data: resolvedType, loading } = useResolvedElementType(
    repo,
    branch,
    element.element_type
  )

  // Content fields = everything except element_type and uuid
  const contentKeys = useMemo(
    () => Object.keys(element).filter((k) => k !== 'element_type' && k !== 'uuid'),
    [element]
  )

  useEffect(() => {
    const el = cardRef.current
    const handle = handleRef.current
    if (!el || !handle || translationMode) return

    const cleanup = combine(
      draggable({
        element: el,
        dragHandle: handle,
        getInitialData: () => ({
          type: 'section-element',
          sectionName,
          uuid: element.uuid,
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
            data.type === 'section-element' &&
            data.sectionName === sectionName &&
            data.uuid !== element.uuid
          )
        },
        getData: ({ input, element: domEl }) =>
          attachClosestEdge(
            { type: 'section-element', sectionName, uuid: element.uuid, index },
            { element: domEl, input, allowedEdges: ['top', 'bottom'] },
          ),
        onDrag: ({ self }) => {
          setClosestEdge(extractClosestEdge(self.data))
        },
        onDragLeave: () => setClosestEdge(null),
        onDrop: () => setClosestEdge(null),
      }),
    )

    return cleanup
  }, [index, sectionName, element.uuid, translationMode])

  function handleFieldChange(fieldName: string, fieldValue: unknown) {
    onUpdate(index, { ...element, [fieldName]: fieldValue })
  }

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
          <span className="font-medium truncate">{element.element_type}</span>
        </button>
        {!translationMode && (
          <button
            type="button"
            onClick={() => onRemove(index)}
            className="p-1 text-red-400 hover:text-red-300 hover:bg-red-500/20 rounded"
            title="Remove element"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      {expanded && (
        <div className="px-4 pb-4 space-y-3 border-t border-white/5 pt-3">
          {loading && <p className="text-xs text-zinc-500">Loading element schema...</p>}

          {resolvedType?.resolved_fields && resolvedType.resolved_fields.length > 0
            ? resolvedType.resolved_fields.map((fieldSchema: any) => {
                const fieldName = fieldSchema.base?.name ?? fieldSchema.name
                if (!fieldName) return null
                return (
                  <ArchetypeFieldRenderer
                    key={fieldName}
                    field={fieldSchema}
                    value={element[fieldName]}
                    onChange={(v) => handleFieldChange(fieldName, v)}
                    translationMode={translationMode}
                    originalValue={originalElement?.[fieldName]}
                    defaultLanguage={defaultLanguage}
                    repo={repo}
                    branch={branch}
                  />
                )
              })
            : contentKeys.map((key) => (
                <div key={key}>
                  <label className="block text-sm font-medium text-zinc-400 mb-1">{key}</label>
                  <input
                    type="text"
                    value={
                      typeof element[key] === 'object'
                        ? JSON.stringify(element[key])
                        : String(element[key] ?? '')
                    }
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
              ))}
        </div>
      )}
    </div>
  )
}

export default function SectionEditor({
  name,
  label,
  value,
  error,
  onChange,
  allowedElementTypes,
  repo,
  branch,
  translationMode,
  originalValue,
  defaultLanguage,
}: SectionEditorProps) {
  const [showTypePicker, setShowTypePicker] = useState(false)

  const items: Element[] = Array.isArray(value) ? value : (value?.items ?? [])

  // Original items list (used as canonical structure in translation mode)
  const originalItems: Element[] = useMemo(() => {
    if (!originalValue) return []
    return Array.isArray(originalValue)
      ? originalValue
      : (originalValue as any)?.items ?? []
  }, [originalValue])

  // In translation mode the ORIGINAL determines which elements exist and their
  // order. Translated field values are overlaid by UUID.
  const displayItems: Element[] = useMemo(() => {
    if (!translationMode || originalItems.length === 0) return items

    const translatedByUuid = new Map<string, Element>()
    for (const el of items) {
      if (el.uuid) translatedByUuid.set(el.uuid, el)
    }

    return originalItems.map(origEl => {
      const translated = translatedByUuid.get(origEl.uuid)
      if (!translated) return origEl
      return { ...origEl, ...translated }
    })
  }, [translationMode, originalItems, items])

  // Build UUID → original element lookup for per-field original hints
  const originalByUuid = useMemo(() => {
    const map = new Map<string, Element>()
    for (const el of originalItems) {
      if (el.uuid) map.set(el.uuid, el)
    }
    return map
  }, [originalItems])

  const updateItems = useCallback(
    (newItems: Element[]) => {
      if (Array.isArray(value)) {
        onChange(newItems)
      } else {
        onChange({
          uuid: value?.uuid ?? crypto.randomUUID(),
          items: newItems,
        })
      }
    },
    [value, onChange],
  )

  function handleAdd(elementType: string) {
    const newElement: Element = {
      element_type: elementType,
      uuid: crypto.randomUUID(),
    }
    updateItems([...items, newElement])
    setShowTypePicker(false)
  }

  const handleUpdate = useCallback(
    (index: number, updated: Element) => {
      if (translationMode) {
        // In translation mode displayItems follows original order.
        // Find/create the element in the actual items array by UUID.
        const uuid = updated.uuid
        const existingIdx = items.findIndex(el => el.uuid === uuid)
        const next = [...items]
        if (existingIdx >= 0) {
          next[existingIdx] = updated
        } else {
          next.push(updated)
        }
        updateItems(next)
      } else {
        const next = [...items]
        next[index] = updated
        updateItems(next)
      }
    },
    [items, updateItems, translationMode],
  )

  const handleRemove = useCallback(
    (index: number) => {
      updateItems(items.filter((_, i) => i !== index))
    },
    [items, updateItems],
  )

  // Monitor drops for this section's elements
  useEffect(() => {
    if (translationMode) return

    const cleanup = monitorForElements({
      canMonitor: ({ source }) => {
        const data = source.data
        return data.type === 'section-element' && data.sectionName === name
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

        updateItems(reordered)
      },
    })

    return cleanup
  }, [items, updateItems, name, translationMode])

  return (
    <div>
      <label className="flex items-center gap-2 text-sm font-medium text-zinc-300 mb-3">
        <Layers className="w-4 h-4 text-purple-400" />
        {label}
      </label>

      <div className="space-y-2">
        {displayItems.map((el, idx) => (
          <ElementCard
            key={el.uuid}
            element={el}
            index={idx}
            sectionName={name}
            originalElement={originalByUuid.get(el.uuid)}
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
        <p className="text-sm text-zinc-500 py-3">No elements yet</p>
      )}

      {/* Add element button */}
      {!translationMode && (
        <div className="mt-3 relative">
          <button
            type="button"
            onClick={() => setShowTypePicker(!showTypePicker)}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-purple-500/20 hover:bg-purple-500/30 text-purple-300 rounded text-sm transition-colors"
          >
            <Plus className="w-3.5 h-3.5" />
            Add Element
          </button>

          {showTypePicker && allowedElementTypes && allowedElementTypes.length > 0 && (
            <div className="absolute z-10 mt-1 bg-zinc-800 border border-white/20 rounded-lg shadow-xl py-1 min-w-[200px]">
              {allowedElementTypes.map((et) => (
                <button
                  key={et}
                  type="button"
                  onClick={() => handleAdd(et)}
                  className="w-full text-left px-3 py-2 text-sm text-zinc-300 hover:bg-white/10 hover:text-white"
                >
                  {et}
                </button>
              ))}
            </div>
          )}

          {showTypePicker && (!allowedElementTypes || allowedElementTypes.length === 0) && (
            <div className="absolute z-10 mt-1 bg-zinc-800 border border-white/20 rounded-lg shadow-xl p-3 min-w-[200px]">
              <input
                type="text"
                placeholder="Enter element type name"
                autoFocus
                className="w-full px-3 py-1.5 bg-white/10 border border-white/20 rounded text-sm text-white focus:outline-none focus:ring-1 focus:ring-purple-500"
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const input = e.currentTarget.value.trim()
                    if (input) handleAdd(input)
                  }
                  if (e.key === 'Escape') setShowTypePicker(false)
                }}
              />
            </div>
          )}
        </div>
      )}

      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}
