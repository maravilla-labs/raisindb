/**
 * Auto-Scroll Hook
 *
 * Handles automatic scrolling when dragging near container edges.
 */

import { useCallback, useRef, useEffect } from 'react';
import { getScrollDirection } from '../utils';
import { DEFAULT_DRAG_DROP_CONFIG } from '../types';

export interface UseAutoScrollOptions {
  /** Distance from edge to trigger scroll */
  threshold?: number;
  /** Scroll speed in pixels per frame */
  speed?: number;
  /** Whether auto-scroll is enabled */
  enabled?: boolean;
}

export interface UseAutoScrollReturn {
  /** Set the scroll container element */
  setContainer: (el: HTMLElement | null) => void;
  /** Start auto-scroll based on cursor position */
  startAutoScroll: (x: number, y: number) => void;
  /** Stop auto-scroll */
  stopAutoScroll: () => void;
  /** Update cursor position during scroll */
  updatePosition: (x: number, y: number) => void;
}

export function useAutoScroll(
  options: UseAutoScrollOptions = {}
): UseAutoScrollReturn {
  const {
    threshold = DEFAULT_DRAG_DROP_CONFIG.scrollThreshold,
    speed = DEFAULT_DRAG_DROP_CONFIG.scrollSpeed,
    enabled = true,
  } = options;

  const containerRef = useRef<HTMLElement | null>(null);
  const positionRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  /**
   * Perform scroll based on current position
   */
  const doScroll = useCallback(() => {
    if (!containerRef.current || !enabled) return;

    const { x, y } = positionRef.current;
    const containerRect = containerRef.current.getBoundingClientRect();
    const { horizontal, vertical } = getScrollDirection(
      x,
      y,
      containerRect,
      threshold
    );

    if (horizontal !== 0) {
      containerRef.current.scrollLeft += horizontal * speed;
    }
    if (vertical !== 0) {
      containerRef.current.scrollTop += vertical * speed;
    }
  }, [threshold, speed, enabled]);

  /**
   * Set the scroll container
   */
  const setContainer = useCallback((el: HTMLElement | null) => {
    containerRef.current = el;
  }, []);

  /**
   * Start auto-scroll interval
   */
  const startAutoScroll = useCallback(
    (x: number, y: number) => {
      if (!enabled) return;

      positionRef.current = { x, y };

      if (!intervalRef.current) {
        intervalRef.current = setInterval(doScroll, 16); // ~60fps
      }
    },
    [enabled, doScroll]
  );

  /**
   * Stop auto-scroll interval
   */
  const stopAutoScroll = useCallback(() => {
    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }
  }, []);

  /**
   * Update cursor position
   */
  const updatePosition = useCallback((x: number, y: number) => {
    positionRef.current = { x, y };
  }, []);

  // Clean up on unmount
  useEffect(() => {
    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, []);

  return {
    setContainer,
    startAutoScroll,
    stopAutoScroll,
    updatePosition,
  };
}
