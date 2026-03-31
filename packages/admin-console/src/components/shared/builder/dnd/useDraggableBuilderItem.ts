/**
 * useDraggableBuilderItem Hook
 *
 * Makes a builder item draggable and a drop target using Pragmatic Drag and Drop.
 * Uses the tree-item hitbox for VS Code-style drop zones.
 * Shows a custom drag preview with icon and label.
 *
 * Adapted from useDraggableTreeNode for builder-specific use cases.
 */

import { useEffect, useRef, type RefObject, type ComponentType } from 'react'
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'
import { disableNativeDragPreview } from '@atlaskit/pragmatic-drag-and-drop/element/disable-native-drag-preview'
import {
  attachInstruction,
  extractInstruction,
  type Instruction,
} from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'
import type { DragState, DropState, BuilderItemDragData } from './types'

export type { Instruction }

// Type for the drag preview context (matches DragPreviewContext.tsx)
interface DragPreviewData {
  itemType: string
  label: string
  Icon: ComponentType<{ className?: string }>
  colorClasses: string
}

interface DragPosition {
  x: number
  y: number
}

interface DragPreviewContextValue {
  preview: DragPreviewData | null
  position: DragPosition | null
  startPreview: (data: DragPreviewData) => void
  updatePosition: (position: DragPosition) => void
  endPreview: () => void
}

interface UseDraggableBuilderItemOptions {
  /** Ref to the DOM element */
  ref: RefObject<HTMLElement | null>
  /** Unique ID of the item */
  id: string
  /** Path to the item */
  path: string
  /** Item type (e.g., "TextField", "CompositeField") */
  itemType: string
  /** Whether this item is a container (can have children) */
  isContainer: boolean
  /** Whether this container is expanded */
  isExpanded?: boolean
  /** Level in the hierarchy (0 = top level) */
  level: number
  /** Whether drag is disabled */
  isDragDisabled?: boolean
  /** Callback when drag state changes */
  onDragStateChange?: (state: DragState) => void
  /** Callback when drop state changes */
  onDropStateChange?: (state: DropState) => void
  /** Display label for the drag preview */
  label?: string
  /** CSS classes for icon colors */
  colorClasses?: string
  /** Icon component for the drag preview */
  Icon?: ComponentType<{ className?: string }>
  /** Optional drag preview context (for custom preview) */
  dragPreviewContext?: DragPreviewContextValue | null
}

export function useDraggableBuilderItem({
  ref,
  id,
  path,
  itemType,
  isContainer,
  isExpanded = false,
  level,
  isDragDisabled = false,
  onDragStateChange,
  onDropStateChange,
  label,
  colorClasses,
  Icon,
  dragPreviewContext,
}: UseDraggableBuilderItemOptions) {
  // Track cleanup functions
  const cleanupRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return

    // Clean up previous
    if (cleanupRef.current) {
      cleanupRef.current()
    }

    if (isDragDisabled) {
      // Clear any existing state when disabled
      onDropStateChange?.({ instruction: null, isDraggedOver: false })
      return
    }

    const dragData: BuilderItemDragData = {
      type: 'builder-item',
      id,
      path,
      itemType,
      isContainer,
      level,
    }

    const cleanup = combine(
      // Make element draggable
      draggable({
        element: el,
        getInitialData: () => dragData as unknown as Record<string, unknown>,
        onGenerateDragPreview: ({ nativeSetDragImage }) => {
          // Disable the native drag preview - we use a custom overlay
          disableNativeDragPreview({ nativeSetDragImage })
        },
        onDragStart: ({ location }) => {
          onDragStateChange?.({ isDragging: true })

          // Start custom preview if context and props are available
          if (dragPreviewContext && Icon && label && colorClasses) {
            dragPreviewContext.startPreview({
              itemType,
              label,
              Icon,
              colorClasses,
            })
            dragPreviewContext.updatePosition({
              x: location.current.input.clientX,
              y: location.current.input.clientY,
            })
          }
        },
        onDrag: ({ location }) => {
          // Update preview position during drag
          if (dragPreviewContext) {
            dragPreviewContext.updatePosition({
              x: location.current.input.clientX,
              y: location.current.input.clientY,
            })
          }
        },
        onDrop: () => {
          onDragStateChange?.({ isDragging: false })

          // End custom preview
          if (dragPreviewContext) {
            dragPreviewContext.endPreview()
          }
        },
      }),

      // Make element a drop target
      dropTargetForElements({
        element: el,
        canDrop: ({ source }) => {
          const sourceData = source.data as unknown as BuilderItemDragData
          // Cannot drop on itself
          if (sourceData.type === 'builder-item' && sourceData.id === id) {
            return false
          }
          // Cannot drop a parent onto its child (path check)
          if (sourceData.type === 'builder-item') {
            const sourcePath = sourceData.path
            if (path.startsWith(sourcePath + '[') || path.startsWith(sourcePath + '.')) {
              return false
            }
          }
          return true
        },
        getData: ({ input, element }) => {
          // Use tree-item hitbox to determine drop position
          // mode: 'expanded' allows dropping inside containers, 'standard' only before/after
          const mode = isContainer ? (isExpanded ? 'expanded' : 'standard') : 'standard'

          return attachInstruction(
            { id, path },
            {
              input,
              element,
              currentLevel: level,
              indentPerLevel: 12,
              mode,
              block: isContainer ? [] : ['make-child'], // Non-containers can't have children
            }
          )
        },
        onDrag: ({ self, source }) => {
          const sourceData = source.data as unknown as BuilderItemDragData
          // Don't show indicator when dragging over self
          if (sourceData.type === 'builder-item' && sourceData.id === id) {
            onDropStateChange?.({ instruction: null, isDraggedOver: false })
            return
          }
          const instruction = extractInstruction(self.data)
          onDropStateChange?.({ instruction, isDraggedOver: true })
        },
        onDragLeave: () => {
          onDropStateChange?.({ instruction: null, isDraggedOver: false })
        },
        onDrop: () => {
          onDropStateChange?.({ instruction: null, isDraggedOver: false })
        },
      })
    )

    cleanupRef.current = cleanup

    return () => {
      cleanup()
      cleanupRef.current = null
    }
  }, [
    ref,
    id,
    path,
    itemType,
    isContainer,
    isExpanded,
    level,
    isDragDisabled,
    onDragStateChange,
    onDropStateChange,
    label,
    colorClasses,
    Icon,
    dragPreviewContext,
  ])
}
