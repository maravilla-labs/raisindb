/**
 * Builder Drop Indicator Component
 *
 * VS Code-style visual indicator for drag-and-drop operations.
 * Shows a line with circles at both ends for reorder (before/after),
 * or a subtle background highlight for dropping into a container (make-child).
 */

import type { Instruction } from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'
import { BUILDER_ITEM_GAP_PX } from './types'

interface BuilderDropIndicatorProps {
  /** The drop instruction from Pragmatic DnD */
  instruction: Instruction
  /** Indentation level (for nested items) */
  level?: number
}

export function BuilderDropIndicator({
  instruction,
  level = 0,
}: BuilderDropIndicatorProps) {
  const indent = level * 12

  // For 'make-child' (dropping into a container), show background highlight
  if (instruction.type === 'make-child') {
    return (
      <div className="absolute inset-0 bg-primary-500/20 rounded pointer-events-none" />
    )
  }

  // For 'reorder-above' or 'reorder-below', show line with circles
  // Position exactly in the center of the gap between items
  // The indicator has height (8px circles), so we use transform to center it
  // at exactly half the gap from the item edge
  const isAbove = instruction.type === 'reorder-above'
  const gapOffset = BUILDER_ITEM_GAP_PX / 2 // Half the gap to center the indicator

  return (
    <div
      className="absolute left-0 right-0 flex items-center pointer-events-none z-10"
      style={{
        marginLeft: `${indent}px`,
        top: isAbove ? -gapOffset : undefined,
        bottom: isAbove ? undefined : -gapOffset,
        // translateY centers the indicator at the gap midpoint
        // -50% for above (moves up by half height), +50% for below (moves down by half height)
        transform: isAbove ? 'translateY(-50%)' : 'translateY(50%)',
      }}
    >
      {/* Left circle */}
      <div className="w-2 h-2 rounded-full bg-primary-500 flex-shrink-0" />

      {/* Line */}
      <div className="flex-1 h-0.5 bg-primary-500" />

      {/* Right circle */}
      <div className="w-2 h-2 rounded-full bg-primary-500 flex-shrink-0" />
    </div>
  )
}
