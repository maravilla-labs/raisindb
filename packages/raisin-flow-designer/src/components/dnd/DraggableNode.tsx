/**
 * Draggable Node Wrapper
 *
 * HOC that adds drag behavior to any node component.
 */

import type { ReactNode } from 'react';
import { clsx } from 'clsx';

export interface DraggableNodeProps {
  /** Node ID for drag identification */
  nodeId: string;
  /** Whether node is being dragged */
  isDragging?: boolean;
  /** Drag handlers from useDragAndDrop */
  dragHandlers?: {
    onPointerDown: (e: React.PointerEvent) => void;
  };
  /** Child node content */
  children: ReactNode;
  /** Custom class name */
  className?: string;
}

export function DraggableNode({
  nodeId,
  isDragging = false,
  dragHandlers,
  children,
  className,
}: DraggableNodeProps) {
  return (
    <div
      data-flow-draggable={nodeId}
      className={clsx(
        'transition-opacity duration-200',
        isDragging && 'opacity-50',
        className
      )}
      {...dragHandlers}
    >
      {children}
    </div>
  );
}
