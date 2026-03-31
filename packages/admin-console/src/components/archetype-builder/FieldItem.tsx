/**
 * Field Item Component
 *
 * Displays a single field in the archetype builder canvas.
 * Uses Pragmatic Drag and Drop for reordering and nesting.
 */

import { useState, useRef, useCallback, useEffect } from 'react'
import { GripVertical, Trash2, CheckCircle, ChevronRight, ChevronDown, Globe } from 'lucide-react'
import { dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { FIELD_TYPE_ICONS, FIELD_TYPE_LABELS, FIELD_TYPE_COLORS } from './constants'
import {
  useDraggableBuilderItem,
  useDragPreviewContext,
  BuilderDropIndicator,
  BUILDER_ITEM_GAP_REM,
  type DragState,
  type DropState,
  type Instruction,
} from '../shared/builder'
import type { FieldSchema } from './types'

interface FieldItemProps {
  field: FieldSchema
  index: number
  level?: number
  path: string // Full path to this field (e.g., "fieldId" or "fieldId[0]")
  selectedPath?: string // Currently selected path
  onPathSelect: (path: string | undefined) => void // Callback for path-based selection
  onPathDelete: (path: string) => void // Callback for path-based deletion
  isDraggable?: boolean // Whether this item can be dragged
}

export default function FieldItem({
  field,
  index: _index,
  level = 0,
  path,
  selectedPath,
  onPathSelect,
  onPathDelete,
  isDraggable = true,
}: FieldItemProps) {
  const Icon = FIELD_TYPE_ICONS[field.$type]
  const label = field.name || FIELD_TYPE_LABELS[field.$type]
  const colors = FIELD_TYPE_COLORS[field.$type]
  const [isExpanded, setIsExpanded] = useState(true)
  const nodeRef = useRef<HTMLDivElement>(null)

  // Drag preview context
  const dragPreviewContext = useDragPreviewContext()

  // Drag and drop state
  const [dragState, setDragState] = useState<DragState>({ isDragging: false })
  const [dropState, setDropState] = useState<DropState>({ instruction: null, isDraggedOver: false })

  // Track last instruction for smooth fade-out (prevents blinking between items)
  const [stickyInstruction, setStickyInstruction] = useState<Instruction | null>(null)
  const [isIndicatorVisible, setIsIndicatorVisible] = useState(false)

  useEffect(() => {
    if (dropState.instruction) {
      // New instruction - show immediately
      setStickyInstruction(dropState.instruction)
      setIsIndicatorVisible(true)
    } else {
      // No instruction - fade out (keep showing stickyInstruction while fading)
      setIsIndicatorVisible(false)
    }
  }, [dropState.instruction])

  // CompositeField has nested fields array
  const isCompositeType = field.$type === 'CompositeField'
  const isDroppableContainer = isCompositeType

  // CompositeField nested fields
  const compositeFields = isCompositeType && Array.isArray(field.fields) ? field.fields : []

  // Check if this path or any child is selected
  const isThisSelected = selectedPath === path
  const hasSelectedChild = selectedPath?.startsWith(path + '.') || selectedPath?.startsWith(path + '[')

  // Use Pragmatic DnD hook with custom drag preview
  useDraggableBuilderItem({
    ref: nodeRef,
    id: field.id!,
    path,
    itemType: field.$type,
    isContainer: isDroppableContainer,
    isExpanded,
    level,
    isDragDisabled: !isDraggable,
    onDragStateChange: setDragState,
    onDropStateChange: setDropState,
    label,
    colorClasses: colors,
    Icon,
    dragPreviewContext,
  })

  // Handle nested container drop zone
  const nestedDropZoneRef = useRef<HTMLDivElement | null>(null)
  const [nestedDropState, setNestedDropState] = useState<DropState>({ instruction: null, isDraggedOver: false })

  // Setup nested drop zone for CompositeField
  const setupNestedDropZone = useCallback((el: HTMLDivElement | null) => {
    if (!el || !isDroppableContainer) return

    return dropTargetForElements({
      element: el,
      canDrop: ({ source }) => {
        const sourceData = source.data as { type?: string; id?: string; path?: string }
        // Cannot drop a parent onto its child
        if (sourceData.type === 'builder-item' && sourceData.path) {
          if (path.startsWith(sourceData.path + '[') || path.startsWith(sourceData.path + '.')) {
            return false
          }
        }
        return true
      },
      getData: () => ({
        id: field.id,
        path,
        // Provide instruction data for the drop monitor
        instruction: { type: 'make-child' },
      }),
      onDragEnter: () => {
        setNestedDropState({ instruction: null, isDraggedOver: true })
      },
      onDragLeave: () => {
        setNestedDropState({ instruction: null, isDraggedOver: false })
      },
      onDrop: () => {
        setNestedDropState({ instruction: null, isDraggedOver: false })
      },
    })
  }, [isDroppableContainer, path, field.id])

  return (
    <div
      className="relative"
      style={{
        marginLeft: level > 0 ? `${level * 1.5}rem` : undefined,
        marginTop: level > 0 ? '0.25rem' : undefined,
        marginBottom: BUILDER_ITEM_GAP_REM,
      }}
    >
      {/* Drop indicator - positioned in outer wrapper for visibility */}
      {/* Uses sticky instruction with fade transition to prevent blinking between items */}
      {stickyInstruction && (
        <div
          className="transition-opacity duration-75"
          style={{ opacity: isIndicatorVisible ? 1 : 0 }}
        >
          <BuilderDropIndicator instruction={stickyInstruction} level={0} />
        </div>
      )}

      <div
        ref={nodeRef}
        className={`
          relative group flex items-center gap-2 p-2 rounded-md border-2 cursor-pointer
          transition-all duration-150
          ${
            isThisSelected
              ? 'bg-primary-500/20 border-primary-400 shadow-lg outline outline-2 outline-primary-500/50'
              : hasSelectedChild
              ? 'bg-primary-500/10 border-primary-400/30'
              : 'bg-white/5 border-white/10 hover:border-primary-400/60 hover:bg-white/10'
          }
          ${dragState.isDragging ? 'shadow-xl opacity-90 border-primary-500 z-50' : ''}
        `}
        onClick={(e) => {
          e.stopPropagation()
          if (isThisSelected) {
            onPathSelect(undefined)
          } else {
            onPathSelect(path)
          }
        }}
      >

        {isDraggable ? (
          <div className="cursor-grab active:cursor-grabbing text-zinc-500 hover:text-zinc-300">
            <GripVertical className="w-3.5 h-3.5" />
          </div>
        ) : (
          <div className="w-3.5" /> // Spacer for alignment
        )}

        {isDroppableContainer && (
          <button
            onClick={(e) => {
              e.stopPropagation()
              setIsExpanded(!isExpanded)
            }}
            className="text-zinc-400 hover:text-white transition-colors"
          >
            {isExpanded ? (
              <ChevronDown className="w-3.5 h-3.5" />
            ) : (
              <ChevronRight className="w-3.5 h-3.5" />
            )}
          </button>
        )}

        <div className={`flex items-center justify-center w-7 h-7 rounded border ${colors}`}>
          <Icon className="w-3.5 h-3.5" />
        </div>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-white truncate">
              {field.name || 'Unnamed Field'}
            </span>
            {field.required && (
              <CheckCircle className="w-3 h-3 text-green-400 flex-shrink-0" />
            )}
            {field.translatable && (
              <Globe className="w-3 h-3 text-blue-400 flex-shrink-0" />
            )}
            {field.multiple && (
              <span className="text-[10px] px-1.5 py-0.5 bg-purple-500/20 text-purple-400 rounded">
                MULTI
              </span>
            )}
          </div>
          <div className="text-xs text-zinc-400">
            {FIELD_TYPE_LABELS[field.$type]}
            {field.label && ` • ${field.label}`}
          </div>
        </div>

        <button
          onClick={(e) => {
            e.stopPropagation()
            onPathDelete(path)
          }}
          className="opacity-0 group-hover:opacity-100 transition-opacity p-1 hover:bg-red-500/20 text-red-400 rounded"
          aria-label="Delete field"
        >
          <Trash2 className="w-3.5 h-3.5" />
        </button>
      </div>

      {/* Nested CompositeField items */}
      {isExpanded && isCompositeType && (
        <div
          ref={(el) => {
            nestedDropZoneRef.current = el
            if (el) setupNestedDropZone(el)
          }}
          className={`mt-1 ml-4 pl-3 border-l-2 min-h-[2rem] transition-colors ${
            nestedDropState.isDraggedOver ? 'border-primary-400 bg-primary-500/10' : 'border-white/10'
          }`}
        >
          {compositeFields.map((nestedField, idx) => {
            // For CompositeField, use index-based path since items are ordered
            const nestedPath = `${path}[${idx}]`
            return (
              <FieldItem
                key={nestedField.id || `${path}-${idx}`}
                field={nestedField}
                index={idx}
                level={level + 1}
                path={nestedPath}
                selectedPath={selectedPath}
                onPathSelect={onPathSelect}
                onPathDelete={onPathDelete}
                isDraggable={true}
              />
            )
          })}
          {compositeFields.length === 0 && (
            <div className={`text-xs py-2 px-2 rounded ${
              nestedDropState.isDraggedOver ? 'text-primary-300' : 'text-zinc-500'
            }`}>
              {nestedDropState.isDraggedOver ? 'Drop here to add' : 'Drop fields here (drag to reorder)'}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
