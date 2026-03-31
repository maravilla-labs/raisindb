/**
 * useDraggableTreeNode Hook
 *
 * Makes a tree node draggable and a drop target using Pragmatic Drag and Drop.
 * Uses the tree-item hitbox for VS Code-style drop zones.
 */

import { useEffect, useRef, type RefObject } from 'react'
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine'
import {
  attachInstruction,
  extractInstruction,
  type Instruction,
} from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'
import type { Node as NodeType } from '../../../../api/nodes'

export type { Instruction }

export interface DragState {
  isDragging: boolean
}

export interface DropState {
  instruction: Instruction | null
  isDraggedOver: boolean
}

interface UseDraggableTreeNodeOptions {
  ref: RefObject<HTMLDivElement | null>
  node: NodeType
  level: number
  isFolder: boolean
  isExpanded: boolean
  isDragDisabled?: boolean
  onDragStateChange?: (state: DragState) => void
  onDropStateChange?: (state: DropState) => void
}

export function useDraggableTreeNode({
  ref,
  node,
  level,
  isFolder,
  isExpanded,
  isDragDisabled = false,
  onDragStateChange,
  onDropStateChange,
}: UseDraggableTreeNodeOptions) {
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

    const cleanup = combine(
      // Make element draggable
      draggable({
        element: el,
        getInitialData: () => ({
          id: node.id,
          path: node.path,
          name: node.name,
          nodeType: node.node_type,
          type: 'tree-item',
          node,
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
          if (source.data.id === node.id) return false
          // Cannot drop a parent onto its child
          const sourcePath = source.data.path as string
          if (node.path.startsWith(sourcePath + '/')) return false
          return true
        },
        getData: ({ input, element }) => {
          // Use tree-item hitbox to determine drop position
          // mode: 'expanded' allows dropping inside folders, 'standard' only before/after
          const mode = isFolder ? (isExpanded ? 'expanded' : 'standard') : 'standard'

          return attachInstruction(
            { id: node.id, path: node.path },
            {
              input,
              element,
              currentLevel: level,
              indentPerLevel: 12,
              mode,
              block: isFolder ? [] : ['make-child'], // Functions can't have children
            }
          )
        },
        onDrag: ({ self, source }) => {
          // Don't show indicator when dragging over self
          if (source.data.id === node.id) {
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
  }, [ref, node, level, isFolder, isExpanded, isDragDisabled, onDragStateChange, onDropStateChange])
}
