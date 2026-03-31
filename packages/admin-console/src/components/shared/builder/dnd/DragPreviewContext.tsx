/**
 * Drag Preview Context
 *
 * Provides global state for custom drag previews.
 * Used to show a compact icon + label preview during toolbox drags.
 */

import { createContext, useContext, useState, useCallback, type ReactNode, type ComponentType } from 'react'

export interface DragPreviewData {
  /** Item type being dragged */
  itemType: string
  /** Display label */
  label: string
  /** Icon component */
  Icon: ComponentType<{ className?: string }>
  /** CSS color classes */
  colorClasses: string
}

export interface DragPosition {
  x: number
  y: number
}

interface DragPreviewContextValue {
  /** Current drag preview data (null if not dragging) */
  preview: DragPreviewData | null
  /** Current cursor position */
  position: DragPosition | null
  /** Start showing a drag preview */
  startPreview: (data: DragPreviewData) => void
  /** Update cursor position */
  updatePosition: (position: DragPosition) => void
  /** Stop showing the drag preview */
  endPreview: () => void
}

const DragPreviewContext = createContext<DragPreviewContextValue | null>(null)

export function DragPreviewProvider({ children }: { children: ReactNode }) {
  const [preview, setPreview] = useState<DragPreviewData | null>(null)
  const [position, setPosition] = useState<DragPosition | null>(null)

  const startPreview = useCallback((data: DragPreviewData) => {
    setPreview(data)
  }, [])

  const updatePosition = useCallback((pos: DragPosition) => {
    setPosition(pos)
  }, [])

  const endPreview = useCallback(() => {
    setPreview(null)
    setPosition(null)
  }, [])

  return (
    <DragPreviewContext.Provider
      value={{
        preview,
        position,
        startPreview,
        updatePosition,
        endPreview,
      }}
    >
      {children}
    </DragPreviewContext.Provider>
  )
}

export function useDragPreviewContext() {
  const context = useContext(DragPreviewContext)
  if (!context) {
    throw new Error('useDragPreviewContext must be used within a DragPreviewProvider')
  }
  return context
}
