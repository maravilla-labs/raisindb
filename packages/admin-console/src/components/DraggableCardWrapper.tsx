/**
 * DraggableCardWrapper Component
 *
 * Wraps any card component to add drag-and-drop reordering capability.
 * Uses useDraggableCard hook for consistent DnD behavior.
 */

import { useRef, useState, type ReactNode } from 'react'
import { useDraggableCard, type DragState, type DropState, type DropPosition } from '../hooks/useDraggableCard'
import { CardDropIndicator } from './CardDropIndicator'

interface DraggableCardWrapperProps {
  /** Unique identifier for the item */
  id: string
  /** Path of the item (for hierarchy checks) */
  path: string
  /** Display name of the item */
  name: string
  /** Type of item: 'folder' | 'user' | 'role' | 'group' */
  type: string
  /** Whether dragging is disabled */
  isDragDisabled?: boolean
  /** The card content to render */
  children: ReactNode
  /** Additional class names */
  className?: string
}

export function DraggableCardWrapper({
  id,
  path,
  name,
  type,
  isDragDisabled = false,
  children,
  className = '',
}: DraggableCardWrapperProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [dragState, setDragState] = useState<DragState>({ isDragging: false })
  const [dropState, setDropState] = useState<DropState>({ position: null, isDraggedOver: false })

  useDraggableCard({
    ref,
    id,
    path,
    name,
    type,
    isDragDisabled,
    onDragStateChange: setDragState,
    onDropStateChange: setDropState,
  })

  return (
    <div
      ref={ref}
      className={`relative ${dragState.isDragging ? 'opacity-50' : ''} ${className}`}
      data-draggable-id={id}
      data-draggable-path={path}
      data-draggable-type={type}
    >
      {children}
      {dropState.isDraggedOver && <CardDropIndicator position={dropState.position} />}
    </div>
  )
}

/**
 * Hook to get the current drop position for an item
 * Used by pages to determine where to insert dropped items
 */
export function getDropPosition(dropState: DropState): DropPosition {
  return dropState.position
}
