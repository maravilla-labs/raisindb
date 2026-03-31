/**
 * Drag Overlay Component
 *
 * Renders a compact drag preview at the cursor position.
 * Uses a portal to render outside the normal DOM hierarchy.
 */

import { createPortal } from 'react-dom'
import { useDragPreviewContext } from './DragPreviewContext'

export function DragOverlay() {
  const { preview, position } = useDragPreviewContext()

  // Don't render if no active drag
  if (!preview || !position) {
    return null
  }

  const { Icon, label, colorClasses } = preview

  // Render the overlay in a portal
  return createPortal(
    <div
      className="fixed pointer-events-none z-[9999]"
      style={{
        left: position.x + 12,
        top: position.y + 12,
      }}
    >
      <div
        className={`
          flex items-center gap-1.5 px-2 py-1 rounded-md border shadow-lg
          backdrop-blur-sm
          ${colorClasses}
        `}
      >
        <Icon className="w-4 h-4 flex-shrink-0" />
        <span className="text-xs font-medium whitespace-nowrap">{label}</span>
      </div>
    </div>,
    document.body
  )
}
