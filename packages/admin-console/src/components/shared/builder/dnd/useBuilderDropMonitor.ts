/**
 * useBuilderDropMonitor Hook
 *
 * Global drop monitor for the builder canvas.
 * Handles drops from both toolbox items and reordering existing items.
 */

import { useEffect } from 'react'
import { monitorForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter'
import { extractInstruction } from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'
import type { DragData, DropResult, DropPosition } from './types'

interface UseBuilderDropMonitorOptions {
  /** Callback when a drop occurs */
  onDrop: (result: DropResult) => void
  /** Whether monitoring is disabled */
  disabled?: boolean
}

/**
 * Convert tree-item instruction to drop position
 */
function instructionToPosition(
  instructionType: string
): DropPosition | null {
  switch (instructionType) {
    case 'reorder-above':
      return 'before'
    case 'reorder-below':
      return 'after'
    case 'make-child':
      return 'inside'
    default:
      return null
  }
}

export function useBuilderDropMonitor({
  onDrop,
  disabled = false,
}: UseBuilderDropMonitorOptions) {
  useEffect(() => {
    if (disabled) return

    const cleanup = monitorForElements({
      onDrop: ({ source, location }) => {
        const destination = location.current.dropTargets[0]
        if (!destination) return

        const sourceData = source.data as unknown as DragData
        const destData = destination.data as {
          id?: string
          path?: string
          instruction?: { type: string }
        }

        // Try tree-item instruction first, then check for synthetic instruction
        const treeInstruction = extractInstruction(destination.data)

        // Determine instruction type - from tree-item hitbox or synthetic
        let instructionType: string | null = null
        if (treeInstruction) {
          instructionType = treeInstruction.type
        } else if (destData.instruction?.type) {
          instructionType = destData.instruction.type
        }

        if (!instructionType) return

        const position = instructionToPosition(instructionType)
        if (!position) return

        const result: DropResult = {
          source: sourceData,
          targetPath: destData.path ?? null,
          position,
        }

        // For inside drops on containers, calculate the index
        // (append to the end by default)
        if (position === 'inside') {
          result.index = undefined // Will be handled by the consumer
        }

        onDrop(result)
      },
    })

    return cleanup
  }, [onDrop, disabled])
}
