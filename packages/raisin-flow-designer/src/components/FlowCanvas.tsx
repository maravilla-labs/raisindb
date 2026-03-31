/**
 * Flow Canvas Component
 *
 * Scrollable and zoomable canvas for the flow designer.
 * Supports light and dark themes with appropriate background and dot patterns.
 */

import { useRef, useCallback, useEffect, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { useThemeClasses } from '../context';

export interface FlowCanvasProps {
  /** Canvas content */
  children: ReactNode;
  /** Current zoom level (0.2 - 3) */
  zoom?: number;
  /** Handler for zoom changes */
  onZoomChange?: (zoom: number) => void;
  /** Minimum zoom level */
  minZoom?: number;
  /** Maximum zoom level */
  maxZoom?: number;
  /** Ref setter for scroll container (for auto-scroll) */
  setScrollContainer?: (el: HTMLElement | null) => void;
  /** Click handler for canvas background (clear selection) */
  onBackgroundClick?: () => void;
  /** Whether pan mode is active */
  panMode?: boolean;
  /** Custom class name */
  className?: string;
}

export function FlowCanvas({
  children,
  zoom = 1,
  onZoomChange,
  minZoom = 0.2,
  maxZoom = 3,
  setScrollContainer,
  onBackgroundClick,
  panMode = false,
  className,
}: FlowCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const isPanningRef = useRef(false);
  const lastPanPosRef = useRef({ x: 0, y: 0 });
  const themeClasses = useThemeClasses();

  // Set scroll container ref
  useEffect(() => {
    setScrollContainer?.(containerRef.current);
    return () => setScrollContainer?.(null);
  }, [setScrollContainer]);

  // Handle wheel zoom
  const handleWheel = useCallback(
    (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        const delta = -e.deltaY * 0.001;
        const newZoom = Math.min(maxZoom, Math.max(minZoom, zoom + delta));
        onZoomChange?.(newZoom);
      }
    },
    [zoom, minZoom, maxZoom, onZoomChange]
  );

  // Add wheel event listener
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    container.addEventListener('wheel', handleWheel, { passive: false });
    return () => container.removeEventListener('wheel', handleWheel);
  }, [handleWheel]);

  // Handle background click
  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === containerRef.current || e.target === contentRef.current) {
        onBackgroundClick?.();
      }
    },
    [onBackgroundClick]
  );

  // Pan mode handlers
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (panMode && e.button === 0) {
        isPanningRef.current = true;
        lastPanPosRef.current = { x: e.clientX, y: e.clientY };
        e.preventDefault();
      }
    },
    [panMode]
  );

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (isPanningRef.current && containerRef.current) {
        const dx = e.clientX - lastPanPosRef.current.x;
        const dy = e.clientY - lastPanPosRef.current.y;
        containerRef.current.scrollLeft -= dx;
        containerRef.current.scrollTop -= dy;
        lastPanPosRef.current = { x: e.clientX, y: e.clientY };
      }
    },
    []
  );

  const handleMouseUp = useCallback(() => {
    isPanningRef.current = false;
  }, []);

  return (
    <div
      ref={containerRef}
      className={clsx(
        'relative overflow-auto',
        themeClasses.canvasBg,
        // Subtle dot pattern (theme-aware)
        themeClasses.canvasDots,
        'bg-[length:24px_24px]',
        panMode ? 'cursor-grab active:cursor-grabbing' : 'cursor-default',
        className
      )}
      onClick={handleClick}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
    >
      <div
        ref={contentRef}
        className="min-w-full min-h-full py-12 px-8"
        style={{
          transform: `scale(${zoom})`,
          transformOrigin: 'top center',
        }}
      >
        {children}
      </div>
    </div>
  );
}
