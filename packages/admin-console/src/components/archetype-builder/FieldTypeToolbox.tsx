/**
 * Field Type Toolbox Component
 *
 * Displays available field types that can be dragged onto the canvas.
 * Uses Pragmatic Drag and Drop for external drag sources.
 */

import { useRef, useState } from 'react'
import { FIELD_TYPES, FIELD_TYPE_ICONS, FIELD_TYPE_LABELS, FIELD_TYPE_COLORS } from './constants'
import { useDraggableToolboxItem, useDragPreviewContext, type DragState } from '../shared/builder'
import type { FieldType } from './types'

interface FieldTypeToolboxProps {
  onAddField?: (type: FieldType) => void
}

interface ToolboxItemProps {
  type: FieldType
  onDoubleClick: () => void
}

function ToolboxItem({ type, onDoubleClick }: ToolboxItemProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [dragState, setDragState] = useState<DragState>({ isDragging: false })
  const dragPreviewContext = useDragPreviewContext()

  const Icon = FIELD_TYPE_ICONS[type]
  const label = FIELD_TYPE_LABELS[type]
  const colors = FIELD_TYPE_COLORS[type]

  useDraggableToolboxItem({
    ref,
    itemType: type,
    label,
    colorClasses: colors,
    Icon,
    onDragStateChange: setDragState,
    dragPreviewContext,
  })

  return (
    <div
      ref={ref}
      onDoubleClick={onDoubleClick}
      className={`
        flex flex-col items-center justify-center p-1.5 rounded-md border cursor-grab active:cursor-grabbing
        transition-all duration-200
        ${colors}
        ${
          dragState.isDragging
            ? 'shadow-lg opacity-90 z-50'
            : 'hover:shadow-md'
        }
      `}
      style={{ userSelect: 'none' }}
    >
      <Icon className="w-4 h-4 mb-0.5" />
      <span className="text-[9px] font-medium text-center leading-tight">
        {FIELD_TYPE_LABELS[type]}
      </span>
    </div>
  )
}

export default function FieldTypeToolbox({ onAddField }: FieldTypeToolboxProps) {
  return (
    <div className="h-full flex flex-col bg-zinc-900/50 border-r border-white/10">
      <div className="px-2 py-2 border-b border-white/10">
        <h3 className="text-[10px] font-semibold text-white uppercase tracking-wide">Types</h3>
      </div>

      <div className="flex-1 overflow-y-auto p-1.5">
        <div
          className="grid gap-1.5"
          style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(56px, 1fr))' }}
        >
          {FIELD_TYPES.map((type) => (
            <ToolboxItem
              key={type}
              type={type}
              onDoubleClick={() => onAddField?.(type)}
            />
          ))}
        </div>
      </div>
    </div>
  )
}
