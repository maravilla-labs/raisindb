/**
 * Custom Drag-and-Drop Hook
 *
 * Implements pointer-based drag-and-drop with:
 * - Long-press (400ms) before drag starts
 * - Distance threshold (5px) to trigger drag
 * - Ghost node with 75% scale, -5° rotation
 * - Auto-scroll during drag (150px threshold)
 * - Drop indicators with before/after/inside positioning
 */

import { useState, useCallback, useRef, useEffect, useLayoutEffect } from 'react';
import type {
  FlowNode,
  DragState,
  DropIndicatorState,
  DragDropConfig,
  InsertPosition,
} from '../types';
import { INITIAL_DRAG_STATE, INITIAL_DROP_INDICATOR_STATE, DEFAULT_DRAG_DROP_CONFIG } from '../types';
import {
  calculateInsertPosition,
  calculateDropIndicator,
  findDropTargetFromPoint,
  getScrollDirection,
  calculateDistance,
} from '../utils';

export interface UseDragAndDropOptions extends Partial<DragDropConfig> {
  onDragStart?: (nodeId: string, node: FlowNode) => void;
  onDragEnd?: (
    sourceId: string,
    targetId: string | null,
    insertPosition: InsertPosition
  ) => void;
  disabled?: boolean;
}

export interface UseDragAndDropReturn {
  dragState: DragState;
  dropIndicator: DropIndicatorState;
  createDragHandlers: (
    nodeId: string,
    node: FlowNode
  ) => {
    onPointerDown: (e: React.PointerEvent) => void;
  };
  setScrollContainer: (el: HTMLElement | null) => void;
  cancelDrag: () => void;
}

export function useDragAndDrop(
  options: UseDragAndDropOptions = {}
): UseDragAndDropReturn {
  const {
    onDragStart,
    onDragEnd,
    disabled = false,
    timeThreshold = DEFAULT_DRAG_DROP_CONFIG.timeThreshold,
    distanceThreshold = DEFAULT_DRAG_DROP_CONFIG.distanceThreshold,
    scrollThreshold = DEFAULT_DRAG_DROP_CONFIG.scrollThreshold,
    scrollSpeed = DEFAULT_DRAG_DROP_CONFIG.scrollSpeed,
  } = options;

  const [dragState, setDragState] = useState<DragState>(INITIAL_DRAG_STATE);
  const [dropIndicator, setDropIndicator] = useState<DropIndicatorState>(
    INITIAL_DROP_INDICATOR_STATE
  );

  // Refs for tracking drag state
  const pointerDownRef = useRef<{ x: number; y: number; time: number } | null>(null);
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dragStartedRef = useRef(false);
  const currentNodeRef = useRef<{ id: string; node: FlowNode } | null>(null);
  const scrollContainerRef = useRef<HTMLElement | null>(null);
  const autoScrollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Refs for stable handler references (fixes stale closure issue)
  const handlePointerMoveRef = useRef<((e: PointerEvent) => void) | null>(null);
  const handlePointerUpRef = useRef<((e: PointerEvent) => void) | null>(null);

  /**
   * Stable wrapper for pointer move (uses ref internally)
   */
  const stablePointerMove = useCallback((e: PointerEvent) => {
    handlePointerMoveRef.current?.(e);
  }, []);

  /**
   * Stable wrapper for pointer up (uses ref internally)
   */
  const stablePointerUp = useCallback((e: PointerEvent) => {
    handlePointerUpRef.current?.(e);
  }, []);

  /**
   * Clean up timers and event listeners
   */
  const cleanup = useCallback(() => {
    if (longPressTimerRef.current) {
      clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
    if (autoScrollIntervalRef.current) {
      clearInterval(autoScrollIntervalRef.current);
      autoScrollIntervalRef.current = null;
    }
    pointerDownRef.current = null;
    dragStartedRef.current = false;
    currentNodeRef.current = null;

    // Use stable wrappers for removal
    window.removeEventListener('pointermove', stablePointerMove);
    window.removeEventListener('pointerup', stablePointerUp);
    window.removeEventListener('pointercancel', stablePointerUp);
  }, [stablePointerMove, stablePointerUp]);

  /**
   * Start the drag operation
   */
  const startDrag = useCallback(
    (x: number, y: number, nodeId: string, node: FlowNode) => {
      dragStartedRef.current = true;
      setDragState({
        isDragging: true,
        draggedNodeId: nodeId,
        draggedNode: node,
        ghostPosition: { x, y },
      });
      onDragStart?.(nodeId, node);

      // Start auto-scroll interval
      if (scrollContainerRef.current) {
        autoScrollIntervalRef.current = setInterval(() => {
          handleAutoScroll();
        }, 16); // ~60fps
      }
    },
    [onDragStart]
  );

  /**
   * Handle auto-scroll during drag
   */
  const handleAutoScroll = useCallback(() => {
    if (!scrollContainerRef.current || !dragState.ghostPosition) return;

    const { x, y } = dragState.ghostPosition;
    const containerRect = scrollContainerRef.current.getBoundingClientRect();
    const { horizontal, vertical } = getScrollDirection(
      x,
      y,
      containerRect,
      scrollThreshold
    );

    if (horizontal !== 0) {
      scrollContainerRef.current.scrollLeft += horizontal * scrollSpeed;
    }
    if (vertical !== 0) {
      scrollContainerRef.current.scrollTop += vertical * scrollSpeed;
    }
  }, [dragState.ghostPosition, scrollThreshold, scrollSpeed]);

  /**
   * Update drag position and drop indicator
   */
  const updateDrag = useCallback((x: number, y: number) => {
    // Update ghost position
    setDragState((prev) => ({
      ...prev,
      ghostPosition: { x, y },
    }));

    const target = findDropTargetFromPoint(x, y);
    const sourceId = currentNodeRef.current?.id;

    if (!target || !sourceId || target.nodeId === sourceId) {
      setDropIndicator((prev) => ({
        ...prev,
        visible: false,
        targetId: null,
      }));
      return;
    }

    // Drop zones (empty container placeholder)
    if (target.dropZoneId) {
      setDropIndicator({
        visible: true,
        orientation: 'horizontal',
        position: { x: target.rect.left, y: target.rect.top },
        size: target.rect.width,
        targetId: target.dropZoneId,
        insertPosition: 'inside',
      });
      return;
    }

    const insertPosition = calculateInsertPosition(x, y, target, sourceId);
    if (!insertPosition) {
      setDropIndicator((prev) => ({
        ...prev,
        visible: false,
        targetId: null,
      }));
      return;
    }

    const indicator = calculateDropIndicator(
      target.nodeId,
      insertPosition,
      target.rect
    );

    setDropIndicator({
      visible: true,
      ...indicator,
    });
  }, []);

  /**
   * Handle pointer move during drag
   */
  const handlePointerMove = useCallback(
    (e: PointerEvent) => {
      if (!pointerDownRef.current || !currentNodeRef.current) return;

      // Calculate distance from start point
      const distance = calculateDistance(
        e.clientX,
        e.clientY,
        pointerDownRef.current.x,
        pointerDownRef.current.y
      );

      // Start drag if moved beyond threshold before long-press timer
      if (!dragStartedRef.current && distance > distanceThreshold) {
        if (longPressTimerRef.current) {
          clearTimeout(longPressTimerRef.current);
          longPressTimerRef.current = null;
        }
        startDrag(
          e.clientX,
          e.clientY,
          currentNodeRef.current.id,
          currentNodeRef.current.node
        );
      }

      // Update during drag
      if (dragStartedRef.current) {
        updateDrag(e.clientX, e.clientY);
      }
    },
    [distanceThreshold, startDrag, updateDrag]
  );

  /**
   * Handle pointer up (end drag)
   */
  const handlePointerUp = useCallback(
    (_e: PointerEvent) => {
      const wasSuccessfulDrag =
        dragStartedRef.current &&
        dropIndicator.targetId &&
        currentNodeRef.current;

      if (wasSuccessfulDrag) {
        onDragEnd?.(
          currentNodeRef.current!.id,
          dropIndicator.targetId,
          dropIndicator.insertPosition
        );
      }

      cleanup();
      setDragState(INITIAL_DRAG_STATE);
      setDropIndicator(INITIAL_DROP_INDICATOR_STATE);
    },
    [dropIndicator.targetId, dropIndicator.insertPosition, onDragEnd, cleanup]
  );

  // Keep refs updated with latest handlers (useLayoutEffect for synchronous update before paint)
  useLayoutEffect(() => {
    handlePointerMoveRef.current = handlePointerMove;
    handlePointerUpRef.current = handlePointerUp;
  }, [handlePointerMove, handlePointerUp]);

  /**
   * Create drag handlers for a specific node
   */
  const createDragHandlers = useCallback(
    (nodeId: string, node: FlowNode) => {
      const handlePointerDown = (e: React.PointerEvent) => {
        if (disabled) return;

        e.stopPropagation();
        pointerDownRef.current = {
          x: e.clientX,
          y: e.clientY,
          time: Date.now(),
        };
        currentNodeRef.current = { id: nodeId, node };
        dragStartedRef.current = false;

        // Start long-press timer
        longPressTimerRef.current = setTimeout(() => {
          if (pointerDownRef.current && !dragStartedRef.current) {
            startDrag(
              pointerDownRef.current.x,
              pointerDownRef.current.y,
              nodeId,
              node
            );
          }
        }, timeThreshold);

        // Add stable handlers (using wrappers that read from refs)
        window.addEventListener('pointermove', stablePointerMove);
        window.addEventListener('pointerup', stablePointerUp);
        window.addEventListener('pointercancel', stablePointerUp);
      };

      return { onPointerDown: handlePointerDown };
    },
    [disabled, timeThreshold, startDrag, stablePointerMove, stablePointerUp]
  );

  /**
   * Cancel the current drag operation
   */
  const cancelDrag = useCallback(() => {
    cleanup();
    setDragState(INITIAL_DRAG_STATE);
    setDropIndicator(INITIAL_DROP_INDICATOR_STATE);
  }, [cleanup]);

  /**
   * Set the scroll container reference
   */
  const setScrollContainer = useCallback((el: HTMLElement | null) => {
    scrollContainerRef.current = el;
  }, []);

  // Clean up on unmount
  useEffect(() => {
    return () => {
      cleanup();
    };
  }, [cleanup]);

  return {
    dragState,
    dropIndicator,
    createDragHandlers,
    setScrollContainer,
    cancelDrag,
  };
}
