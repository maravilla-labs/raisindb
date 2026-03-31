import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { Plus, Trash2, GripVertical, List } from 'lucide-react'
import {
  draggable,
  dropTargetForElements,
  monitorForElements,
} from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'
import {
  attachClosestEdge,
  extractClosestEdge,
  type Edge,
} from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { reorderWithEdge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/util/reorder-with-edge'

interface MultipleFieldWrapperProps {
  name: string
  label: string
  value: unknown[] | undefined
  onChange: (value: unknown[]) => void
  renderItem: (
    itemValue: unknown,
    onItemChange: (v: unknown) => void,
    index: number
  ) => React.ReactNode
  /** Returns default value for new items (e.g., '', 0, false) */
  getDefaultValue?: () => unknown
  translationMode?: boolean
  /** Original (source-language) array for translation reference */
  originalValue?: unknown[]
  error?: string
}

/** Drop indicator line with circles at edges */
function DropIndicator({ edge }: { edge: Edge }) {
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

/** Single item card with drag handle and remove button */
function ItemCard({
  index,
  fieldName,
  onRemove,
  translationMode,
  children,
}: {
  index: number
  fieldName: string
  onRemove: (index: number) => void
  translationMode?: boolean
  children: React.ReactNode
}) {
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
          type: 'multiple-field-item',
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
            data.type === 'multiple-field-item' &&
            data.fieldName === fieldName &&
            data.index !== index
          )
        },
        getData: ({ input, element }) =>
          attachClosestEdge(
            { type: 'multiple-field-item', fieldName, index },
            { element, input, allowedEdges: ['top', 'bottom'] }
          ),
        onDrag: ({ self }) => {
          setClosestEdge(extractClosestEdge(self.data))
        },
        onDragLeave: () => setClosestEdge(null),
        onDrop: () => setClosestEdge(null),
      })
    )

    return cleanup
  }, [index, fieldName, translationMode])

  return (
    <div
      ref={cardRef}
      className="relative flex items-start gap-2 p-2 bg-white/5 border border-white/10 rounded-lg"
      style={{ opacity: isDragging ? 0.4 : 1 }}
    >
      {closestEdge && <DropIndicator edge={closestEdge} />}
      {!translationMode && (
        <div ref={handleRef} className="flex-shrink-0 cursor-grab mt-2">
          <GripVertical className="w-4 h-4 text-zinc-500" />
        </div>
      )}
      <div className="flex-1 min-w-0">{children}</div>
      {!translationMode && (
        <button
          type="button"
          onClick={() => onRemove(index)}
          className="flex-shrink-0 p-1 text-red-400 hover:text-red-300 hover:bg-red-500/20 rounded mt-1"
          title="Remove item"
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  )
}

export default function MultipleFieldWrapper({
  name,
  label,
  value,
  onChange,
  renderItem,
  getDefaultValue = () => '',
  translationMode,
  originalValue,
  error,
}: MultipleFieldWrapperProps) {
  const items: unknown[] = Array.isArray(value) ? value : []

  // In translation mode, the ORIGINAL defines the array length and order.
  // Translated values overlay by index.
  const displayItems: unknown[] = useMemo(() => {
    if (!translationMode || !Array.isArray(originalValue)) return items

    return originalValue.map((origItem, idx) => {
      const translated = items[idx]
      // Use translated value if it exists, otherwise show original
      return translated !== undefined ? translated : origItem
    })
  }, [translationMode, originalValue, items])

  const handleUpdate = useCallback(
    (index: number, updated: unknown) => {
      // Extend items array if needed (original may have more items than
      // the current translation value)
      const next = [...items]
      while (next.length <= index) next.push(getDefaultValue())
      next[index] = updated
      onChange(next)
    },
    [items, onChange, getDefaultValue]
  )

  const handleRemove = useCallback(
    (index: number) => {
      onChange(items.filter((_, i) => i !== index))
    },
    [items, onChange]
  )

  function handleAdd() {
    onChange([...items, getDefaultValue()])
  }

  // Monitor drops for this field's items
  useEffect(() => {
    if (translationMode) return

    const cleanup = monitorForElements({
      canMonitor: ({ source }) => {
        const data = source.data
        return data.type === 'multiple-field-item' && data.fieldName === name
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
  }, [items, onChange, name, translationMode])

  return (
    <div>
      <label className="flex items-center gap-2 text-sm font-medium text-zinc-300 mb-2">
        <List className="w-4 h-4 text-cyan-400" />
        {label}
      </label>

      <div className="space-y-2">
        {displayItems.map((item, idx) => (
          <ItemCard
            key={idx}
            index={idx}
            fieldName={name}
            onRemove={handleRemove}
            translationMode={translationMode}
          >
            {renderItem(item, (v) => handleUpdate(idx, v), idx)}
          </ItemCard>
        ))}
      </div>

      {displayItems.length === 0 && (
        <p className="text-sm text-zinc-500 py-2">No items yet</p>
      )}

      {!translationMode && (
        <button
          type="button"
          onClick={handleAdd}
          className="flex items-center gap-1.5 px-3 py-1.5 bg-cyan-500/20 hover:bg-cyan-500/30 text-cyan-300 rounded text-sm transition-colors mt-2"
        >
          <Plus className="w-3.5 h-3.5" />
          Add Item
        </button>
      )}

      {error && <p className="mt-1 text-sm text-red-400">{error}</p>}
    </div>
  )
}
