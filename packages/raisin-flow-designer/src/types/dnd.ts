/**
 * Drag-and-Drop Type Definitions
 *
 * Types for the custom pointer-based drag-and-drop system.
 */

import type { FlowNode } from './flow';

/** Current drag operation state */
export interface DragState {
  /** Whether a drag is currently in progress */
  isDragging: boolean;
  /** ID of the node being dragged */
  draggedNodeId: string | null;
  /** Reference to the node being dragged */
  draggedNode: FlowNode | null;
  /** Current position of the ghost element */
  ghostPosition: { x: number; y: number } | null;
}

/** Initial drag state */
export const INITIAL_DRAG_STATE: DragState = {
  isDragging: false,
  draggedNodeId: null,
  draggedNode: null,
  ghostPosition: null,
};

/** Drop indicator orientation */
export type DropOrientation = 'horizontal' | 'vertical';

/** Where to insert relative to target */
export type InsertPosition = 'before' | 'after' | 'inside' | 'left' | 'right';

/** Visual drop indicator state */
export interface DropIndicatorState {
  /** Whether indicator is visible */
  visible: boolean;
  /** Line orientation */
  orientation: DropOrientation;
  /** Screen position */
  position: { x: number; y: number };
  /** Line size (width or height depending on orientation) */
  size: number;
  /** ID of target node */
  targetId: string | null;
  /** Where to insert relative to target */
  insertPosition: InsertPosition;
}

/** Initial drop indicator state */
export const INITIAL_DROP_INDICATOR_STATE: DropIndicatorState = {
  visible: false,
  orientation: 'horizontal',
  position: { x: 0, y: 0 },
  size: 0,
  targetId: null,
  insertPosition: 'after',
};

/** Pointer down event data for tracking drag initialization */
export interface PointerDownData {
  /** Initial X coordinate */
  x: number;
  /** Initial Y coordinate */
  y: number;
  /** Timestamp when pointer went down */
  time: number;
}

/** Configuration options for drag-and-drop behavior */
export interface DragDropConfig {
  /** Time in ms before drag starts (long-press) */
  timeThreshold: number;
  /** Distance in px to trigger drag without long-press */
  distanceThreshold: number;
  /** Distance from edge in px to trigger auto-scroll */
  scrollThreshold: number;
  /** Speed of auto-scroll in px per tick */
  scrollSpeed: number;
}

/** Default drag-and-drop configuration */
export const DEFAULT_DRAG_DROP_CONFIG: DragDropConfig = {
  timeThreshold: 400,     // 400ms long-press
  distanceThreshold: 5,   // 5px movement to trigger
  scrollThreshold: 150,   // 150px from edge
  scrollSpeed: 10,        // 10px per tick
};

/** Drop target information from DOM element */
export interface DropTarget {
  /** Target element */
  element: HTMLElement;
  /** Target node ID */
  nodeId: string;
  /** Bounding rect of target */
  rect: DOMRect;
  /** Whether target is a container */
  isContainer: boolean;
  /** Parent container ID */
  parentId: string | null;
  /** Next sibling ID */
  nextId: string | null;
  /** Drop zone target ID (used for inside highlighting) */
  dropZoneId?: string | null;
}
