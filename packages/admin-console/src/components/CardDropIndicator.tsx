/**
 * Card Drop Indicator Component
 *
 * Visual indicator for drag-and-drop operations in grid layouts.
 * Shows a vertical line on the left or right edge of the card.
 */

import type { DropPosition } from '../hooks/useDraggableCard'

interface CardDropIndicatorProps {
  /** The drop position from drag state */
  position: DropPosition
}

export function CardDropIndicator({ position }: CardDropIndicatorProps) {
  if (!position) return null

  const isLeft = position === 'before'

  return (
    <div
      className={`absolute top-0 bottom-0 w-1 bg-primary-400 pointer-events-none z-10 ${
        isLeft ? 'left-0 -translate-x-1/2' : 'right-0 translate-x-1/2'
      }`}
    >
      {/* Top circle */}
      <div className="absolute -top-1 left-1/2 -translate-x-1/2 w-2 h-2 rounded-full bg-primary-400" />
      {/* Bottom circle */}
      <div className="absolute -bottom-1 left-1/2 -translate-x-1/2 w-2 h-2 rounded-full bg-primary-400" />
    </div>
  )
}
