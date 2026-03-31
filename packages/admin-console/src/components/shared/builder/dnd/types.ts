/**
 * Drag and Drop Types for Visual Builders
 *
 * Types for Pragmatic Drag and Drop integration in builders.
 */

import type { Instruction } from '@atlaskit/pragmatic-drag-and-drop-hitbox/tree-item'

/** Re-export Instruction type for convenience */
export type { Instruction }

/**
 * Layout constants for builder items
 * These values are used for consistent spacing and indicator positioning
 */
export const BUILDER_ITEM_GAP_PX = 6 // Gap between items (marginBottom in px)
export const BUILDER_ITEM_GAP_REM = '0.375rem' // Same as above in rem

/** Data attached to draggable builder items */
export interface BuilderItemDragData {
  /** Type discriminator */
  type: 'builder-item'
  /** Unique ID of the item */
  id: string
  /** Path to the item (e.g., "fieldId" or "fieldId[0]") */
  path: string
  /** Item type (e.g., "TextField", "CompositeField", "String", "Object") */
  itemType: string
  /** Whether this item is a container (can have children) */
  isContainer: boolean
  /** Level in the hierarchy (0 = top level) */
  level: number
}

/** Data attached to toolbox items */
export interface ToolboxItemDragData {
  /** Type discriminator */
  type: 'toolbox-item'
  /** The type of item to create (e.g., "TextField", "String") */
  itemType: string
}

/** Union of all drag data types */
export type DragData = BuilderItemDragData | ToolboxItemDragData

/** Drop position within a target */
export type DropPosition = 'before' | 'after' | 'inside'

/** Result of a drop operation */
export interface DropResult {
  /** Source drag data */
  source: DragData
  /** Target item path (null if dropping at root) */
  targetPath: string | null
  /** Where to insert relative to target */
  position: DropPosition
  /** Index within the target container (for inside drops) */
  index?: number
}

/** Drag state for a draggable item */
export interface DragState {
  isDragging: boolean
}

/** Drop state for a droppable item */
export interface DropState {
  instruction: Instruction | null
  isDraggedOver: boolean
}
