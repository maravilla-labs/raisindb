/**
 * Drop Indicator Component
 *
 * VS Code-style visual indicator for drag-and-drop operations.
 * Shows a line with circles at both ends for reorder (before/after),
 * or a subtle background highlight for dropping into a folder (make-child).
 */

import type { Instruction } from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'

interface DropIndicatorProps {
  /** The drop instruction from Pragmatic DnD */
  instruction: Instruction
  /** Indentation level (for nested items) */
  level?: number
}

export function DropIndicator({ instruction, level = 0 }: DropIndicatorProps) {
  const indent = level * 12 + 8

  // For 'make-child' (dropping into a folder), show background highlight
  if (instruction.type === 'make-child') {
    return (
      <div className="absolute inset-0 bg-primary-500/20 rounded pointer-events-none" />
    )
  }

  // For 'reorder-above' or 'reorder-below', show line with circles
  const isAbove = instruction.type === 'reorder-above'

  return (
    <div
      className="absolute left-0 right-0 flex items-center pointer-events-none z-10"
      style={{
        marginLeft: `${indent}px`,
        top: isAbove ? -1 : undefined,
        bottom: isAbove ? undefined : -1,
      }}
    >
      {/* Left circle */}
      <div className="w-2 h-2 rounded-full bg-primary-400 -ml-1 flex-shrink-0" />

      {/* Line */}
      <div className="flex-1 h-0.5 bg-primary-400" />

      {/* Right circle */}
      <div className="w-2 h-2 rounded-full bg-primary-400 -mr-1 flex-shrink-0" />
    </div>
  )
}

/**
 * Alternative: Instruction indicator for debugging
 */
export function DropIndicatorDebug({ instruction, level = 0 }: DropIndicatorProps) {
  return (
    <div
      className="absolute right-2 top-0 text-xs bg-black/80 text-white px-1 rounded"
      style={{ marginLeft: level * 12 }}
    >
      {instruction.type}
    </div>
  )
}
