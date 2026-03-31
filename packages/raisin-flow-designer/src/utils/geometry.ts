/**
 * Geometry Utilities
 *
 * Functions for calculating drop zones and positions.
 */

import type { DropIndicatorState, InsertPosition, DropTarget } from '../types';

/** Drop zone regions within a target element */
export interface DropZoneRegion {
  /** Left boundary percentage (0-1) */
  left: number;
  /** Right boundary percentage (0-1) */
  right: number;
  /** Insert position for this region */
  position: InsertPosition;
}

/** Default drop zone configuration */
export const DROP_ZONES: DropZoneRegion[] = [
  { left: 0, right: 0.25, position: 'before' },    // Left 25% = before
  { left: 0.25, right: 0.75, position: 'inside' }, // Middle 50% = inside (for containers)
  { left: 0.75, right: 1, position: 'after' },     // Right 25% = after
];

/**
 * Calculate insert position based on cursor position relative to element
 * For vertical flow layouts: upper half = before, lower half = after
 * @param sourceId - The ID of the node being dragged, or null for external drops
 */
export function calculateInsertPosition(
  x: number,
  y: number,
  target: DropTarget,
  sourceId: string | null
): InsertPosition | null {
  const { rect, isContainer } = target;
  const relX = (x - rect.left) / rect.width;
  const relY = (y - rect.top) / rect.height;
  // For external drops (sourceId is null), skip parent/sibling checks
  const isParentOfTarget = sourceId ? target.parentId === sourceId : false;
  const isTargetNextSibling = sourceId ? target.nextId === sourceId : false;

  if (relX <= 0.25 && !isParentOfTarget) {
    return 'left';
  }

  if (relX >= 0.75 && !isTargetNextSibling) {
    return 'right';
  }

  if (isContainer) {
    if (relY <= 0.25 && !isParentOfTarget) {
      return 'before';
    }
    if (relY >= 0.75 && !isTargetNextSibling) {
      return 'after';
    }
    return 'inside';
  }

  if (relY <= 0.5 && !isParentOfTarget) {
    return 'before';
  }

  if (!isTargetNextSibling) {
    return 'after';
  }

  return null;
}

/**
 * Calculate drop indicator position and dimensions
 * For vertical flow: before/after show horizontal lines above/below the node
 */
export function calculateDropIndicator(
  targetId: string,
  insertPosition: InsertPosition,
  rect: DOMRect
): Omit<DropIndicatorState, 'visible'> {
  const padding = 20; // Extra space for indicator
  const indicatorOffset = 15; // Distance from element edge

  switch (insertPosition) {
    case 'left':
      return {
        orientation: 'vertical',
        position: { x: rect.left - indicatorOffset, y: rect.top - padding },
        size: rect.height + padding * 2,
        targetId,
        insertPosition,
      };

    case 'right':
      return {
        orientation: 'vertical',
        position: { x: rect.right + indicatorOffset, y: rect.top - padding },
        size: rect.height + padding * 2,
        targetId,
        insertPosition,
      };

    case 'before':
      // Horizontal line above the node
      return {
        orientation: 'horizontal',
        position: { x: rect.left - padding, y: rect.top - indicatorOffset },
        size: rect.width + padding * 2,
        targetId,
        insertPosition,
      };

    case 'after':
      // Horizontal line below the node
      return {
        orientation: 'horizontal',
        position: { x: rect.left - padding, y: rect.bottom + indicatorOffset },
        size: rect.width + padding * 2,
        targetId,
        insertPosition,
      };

    case 'inside':
      // Horizontal line inside container (near top)
      return {
        orientation: 'horizontal',
        position: { x: rect.left, y: rect.top + 50 },
        size: rect.width,
        targetId,
        insertPosition,
      };
  }
}

/**
 * Find drop target element from cursor position
 * Uses elementsFromPoint to handle ghost nodes that may be under the cursor
 */
export function findDropTargetFromPoint(
  x: number,
  y: number
): DropTarget | null {
  // Use elementsFromPoint to get all elements at cursor position
  // This allows us to skip the ghost node that's rendered at cursor during internal drag
  const elements = document.elementsFromPoint(x, y);

  for (const element of elements) {
    // Skip the ghost node (has data-flow-ghost attribute)
    if (element.closest('[data-flow-ghost]')) continue;

    // Check for drop zone
    const dropZone = element.closest('[data-flow-drop-zone]') as HTMLElement | null;
    if (dropZone) {
      const container = dropZone.closest('[data-flow-node-id]') as HTMLElement | null;
      if (container?.dataset.flowNodeId) {
        return {
          element: container,
          nodeId: container.dataset.flowNodeId,
          rect: dropZone.getBoundingClientRect(),
          isContainer: container.dataset.flowNodeContainer === 'true',
          parentId: container.dataset.flowNodeParentId ?? null,
          nextId: container.dataset.flowNodeNextId ?? null,
          dropZoneId: dropZone.dataset.flowDropZone ?? container.dataset.flowNodeId,
        };
      }
    }

    // Check for direct node
    const droppable = element.closest('[data-flow-node-id]') as HTMLElement | null;
    if (droppable?.dataset.flowNodeId) {
      return {
        element: droppable,
        nodeId: droppable.dataset.flowNodeId,
        rect: droppable.getBoundingClientRect(),
        isContainer: droppable.dataset.flowNodeContainer === 'true',
        parentId: droppable.dataset.flowNodeParentId ?? null,
        nextId: droppable.dataset.flowNodeNextId ?? null,
      };
    }
  }

  return null;
}

/**
 * Check if a point is within scroll threshold of container edges
 */
export function getScrollDirection(
  x: number,
  y: number,
  containerRect: DOMRect,
  threshold: number
): { horizontal: number; vertical: number } {
  let horizontal = 0;
  let vertical = 0;

  // Horizontal scroll
  if (x < containerRect.left + threshold) {
    horizontal = -1; // Scroll left
  } else if (x > containerRect.right - threshold) {
    horizontal = 1; // Scroll right
  }

  // Vertical scroll
  if (y < containerRect.top + threshold) {
    vertical = -1; // Scroll up
  } else if (y > containerRect.bottom - threshold) {
    vertical = 1; // Scroll down
  }

  return { horizontal, vertical };
}

/**
 * Calculate distance between two points
 */
export function calculateDistance(
  x1: number,
  y1: number,
  x2: number,
  y2: number
): number {
  const dx = x2 - x1;
  const dy = y2 - y1;
  return Math.sqrt(dx * dx + dy * dy);
}
