/**
 * useDraggableToolboxItem Hook
 *
 * Makes a toolbox item draggable using Pragmatic Drag and Drop.
 * Toolbox items are external sources that create new items when dropped.
 * Shows a custom drag preview with icon and label.
 */

import { useEffect, useRef, type RefObject, type ComponentType } from 'react'
import { draggable } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { disableNativeDragPreview } from '@atlaskit/pragmatic-drag-and-drop/element/disable-native-drag-preview'
import type { DragState, ToolboxItemDragData } from './types'

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

interface UseDraggableToolboxItemOptions {
  /** Ref to the DOM element */
  ref: RefObject<HTMLElement | null>
  /** The type of item this will create (e.g., "TextField", "String") */
  itemType: string
  /** Display label for the drag preview */
  label?: string
  /** CSS classes for icon colors */
  colorClasses?: string
  /** Icon component for the drag preview */
  Icon?: ComponentType<{ className?: string }>
  /** Whether drag is disabled */
  isDragDisabled?: boolean
  /** Callback when drag state changes */
  onDragStateChange?: (state: DragState) => void
  /** Optional drag preview context (for custom preview) */
  dragPreviewContext?: DragPreviewContextValue | null
}

export function useDraggableToolboxItem({
  ref,
  itemType,
  label,
  colorClasses,
  Icon,
  isDragDisabled = false,
  onDragStateChange,
  dragPreviewContext,
}: UseDraggableToolboxItemOptions) {
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
      return
    }

    const dragData: ToolboxItemDragData = {
      type: 'toolbox-item',
      itemType,
    }

    const cleanup = draggable({
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
    })

    cleanupRef.current = cleanup

    return () => {
      cleanup()
      cleanupRef.current = null
    }
  }, [ref, itemType, label, colorClasses, Icon, isDragDisabled, onDragStateChange, dragPreviewContext])
}
