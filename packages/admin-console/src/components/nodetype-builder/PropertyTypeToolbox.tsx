/**
 * Property Type Toolbox Component
 *
 * Displays available property types that can be dragged onto the canvas.
 * Uses Pragmatic Drag and Drop for external drag sources.
 */

import { useRef, useState } from 'react'
import { PROPERTY_TYPES, PROPERTY_TYPE_ICONS, PROPERTY_TYPE_LABELS, PROPERTY_TYPE_COLORS } from './constants'
import { useDraggableToolboxItem, useDragPreviewContext, type DragState } from '../shared/builder'
import type { PropertyType } from './types'

interface PropertyTypeToolboxProps {
  onAddProperty?: (type: PropertyType) => void
}

interface ToolboxItemProps {
  type: PropertyType
  onDoubleClick: () => void
}

function ToolboxItem({ type, onDoubleClick }: ToolboxItemProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [dragState, setDragState] = useState<DragState>({ isDragging: false })
  const dragPreviewContext = useDragPreviewContext()

  const Icon = PROPERTY_TYPE_ICONS[type]
  const label = PROPERTY_TYPE_LABELS[type]
  const colors = PROPERTY_TYPE_COLORS[type]

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
        {PROPERTY_TYPE_LABELS[type]}
      </span>
    </div>
  )
}

export default function PropertyTypeToolbox({ onAddProperty }: PropertyTypeToolboxProps) {
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
          {PROPERTY_TYPES.map((type) => (
            <ToolboxItem
              key={type}
              type={type}
              onDoubleClick={() => onAddProperty?.(type)}
            />
          ))}
        </div>
      </div>
    </div>
  )
}
