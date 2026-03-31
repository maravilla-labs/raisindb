/**
 * Ghost Node Component
 *
 * Drag preview with 75% scale and -5° rotation.
 */

import { clsx } from 'clsx';
import type { FlowNode } from '../../types';
import { isFlowContainer } from '../../types';

export interface GhostNodeProps {
  /** The node being dragged */
  node: FlowNode;
  /** Current position */
  position: { x: number; y: number };
  /** Custom class name */
  className?: string;
}

export function GhostNode({ node, position, className }: GhostNodeProps) {
  const isContainer = isFlowContainer(node);
  const label = isContainer
    ? `${node.container_type.toUpperCase()} Container`
    : node.properties?.action || 'Step';

  return (
    <div
      data-flow-ghost="true"
      className={clsx(
        'fixed pointer-events-none z-[9999]',
        'px-4 py-3 rounded-lg shadow-lg',
        'text-gray-50 font-medium',
        'transform -rotate-[5deg] scale-75',
        'backdrop-blur-xl outline-0 transition-none',
        className
      )}
      style={{
        left: position.x + 10,
        top: position.y + 10,
        backgroundColor: '#3b5998',
      }}
    >
      <span className="truncate max-w-[200px] block">{label}</span>
    </div>
  );
}
