/**
 * useDraggableCard Hook
 *
 * Makes a grid card draggable and a drop target using Pragmatic Drag and Drop.
 * Simplified version of useDraggableTreeNode for grid/list layouts.
 */

import { useEffect, useRef, type RefObject } from 'react'
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'

export type DropPosition = 'before' | 'after' | null

export interface DragData {
  id: string
  path: string
  name: string
  type: string  // 'folder' | 'user' | 'role' | 'group'
  [key: string]: unknown  // Index signature for Record<string, unknown> compatibility
}

export interface DragState {
  isDragging: boolean
}

export interface DropState {
  position: DropPosition
  isDraggedOver: boolean
}

interface UseDraggableCardOptions {
  ref: RefObject<HTMLDivElement | null>
  id: string
  path: string
  name: string
  type: string
  isDragDisabled?: boolean
  onDragStateChange?: (state: DragState) => void
  onDropStateChange?: (state: DropState) => void
}

export function useDraggableCard({
  ref,
  id,
  path,
  name,
  type,
  isDragDisabled = false,
  onDragStateChange,
  onDropStateChange,
}: UseDraggableCardOptions) {
  const cleanupRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return

    // Clean up previous
    if (cleanupRef.current) {
      cleanupRef.current()
    }

    if (isDragDisabled) {
      onDropStateChange?.({ position: null, isDraggedOver: false })
      return
    }

    const cleanup = combine(
      // Make element draggable
      draggable({
        element: el,
        getInitialData: (): DragData => ({
          id,
          path,
          name,
          type,
        }),
        onDragStart: () => {
          onDragStateChange?.({ isDragging: true })
        },
        onDrop: () => {
          onDragStateChange?.({ isDragging: false })
        },
      }),

      // Make element a drop target
      dropTargetForElements({
        element: el,
        canDrop: ({ source }) => {
          // Cannot drop on itself
          if (source.data.id === id) return false
          // Cannot drop a parent onto its child (for folders)
          const sourcePath = source.data.path as string
          if (path.startsWith(sourcePath + '/')) return false
          // Only allow reordering within same type or folders
          return true
        },
        getData: () => ({
          id,
          path,
          type,
        }),
        onDrag: ({ self, source, location }) => {
          // Don't show indicator when dragging over self
          if (source.data.id === id) {
            onDropStateChange?.({ position: null, isDraggedOver: false })
            return
          }

          // Determine position based on mouse location relative to element
          const element = self.element
          const rect = element.getBoundingClientRect()
          const mouseX = location.current.input.clientX
          const midpoint = rect.left + rect.width / 2

          const position: DropPosition = mouseX < midpoint ? 'before' : 'after'
          onDropStateChange?.({ position, isDraggedOver: true })
        },
        onDragLeave: () => {
          onDropStateChange?.({ position: null, isDraggedOver: false })
        },
        onDrop: () => {
          onDropStateChange?.({ position: null, isDraggedOver: false })
        },
      })
    )

    cleanupRef.current = cleanup

    return () => {
      cleanup()
      cleanupRef.current = null
    }
  }, [ref, id, path, name, type, isDragDisabled, onDragStateChange, onDropStateChange])
}
