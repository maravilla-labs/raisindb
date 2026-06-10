/**
 * Error Edge Overlay
 *
 * Absolutely-positioned SVG overlay (inside the scaled canvas content)
 * that draws red dashed elbow connectors on the RIGHT side of the canvas
 * between steps and their error handler nodes.
 *
 * Pure DOM measurement (no graph library): node positions are resolved
 * via their `data-flow-node-id` attributes relative to the overlay's own
 * bounding rect. Because the overlay lives inside the zoom-scaled
 * container, measured (scaled) deltas are divided by the effective scale
 * so coordinates stay correct at any zoom level.
 */

import { useLayoutEffect, useRef, useState } from 'react';
import { clsx } from 'clsx';
import { AlertTriangle } from 'lucide-react';
import type { FlowDefinition, FlowNode, FlowStep } from '../../types';
import { isFlowStep, isFlowContainer } from '../../types';
import { getErrorEdge, getAllNodeIds } from '../../utils';

/** A logical error edge (source step -> error handler node) */
interface ErrorEdgeDef {
  sourceId: string;
  targetId: string;
}

/** A measured error edge in local (unscaled) overlay coordinates */
interface MeasuredEdge extends ErrorEdgeDef {
  /** SVG path data for the elbow connector */
  path: string;
  /** Label badge anchor (centered on the vertical rail) */
  labelX: number;
  labelY: number;
}

export interface ErrorEdgeOverlayProps {
  /** Flow definition to extract error edges from */
  flow: FlowDefinition;
  /** Node IDs that failed during execution (their edges render active) */
  failedNodeIds?: Set<string>;
  /** Custom class name */
  className?: string;
}

/** Horizontal gap between the rightmost node edge and the first rail */
const RAIL_OFFSET = 32;
/** Horizontal spacing between parallel rails */
const RAIL_SPACING = 18;
/** Gap between arrow tip and the target node edge */
const ARROW_GAP = 4;

/**
 * Collect all steps that have an error edge pointing at another
 * existing node in the flow.
 */
function collectErrorEdges(flow: FlowDefinition): ErrorEdgeDef[] {
  const validIds = new Set(getAllNodeIds(flow.nodes));
  const edges: ErrorEdgeDef[] = [];

  const walk = (nodes: FlowNode[]) => {
    for (const node of nodes) {
      if (isFlowStep(node)) {
        const target = getErrorEdge(node as FlowStep);
        if (target && target !== node.id && validIds.has(target)) {
          edges.push({ sourceId: node.id, targetId: target });
        }
      }
      if (isFlowContainer(node)) {
        walk(node.children);
      }
    }
  };
  walk(flow.nodes);

  return edges;
}

export function ErrorEdgeOverlay({
  flow,
  failedNodeIds,
  className,
}: ErrorEdgeOverlayProps) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState<MeasuredEdge[]>([]);

  useLayoutEffect(() => {
    const wrapper = wrapperRef.current;
    if (!wrapper) return;

    const measure = () => {
      const defs = collectErrorEdges(flow);
      if (defs.length === 0) {
        setEdges((prev) => (prev.length === 0 ? prev : []));
        return;
      }

      // Search within the canvas content (overlay's containing block)
      const root = wrapper.parentElement ?? wrapper;
      const wrapperRect = wrapper.getBoundingClientRect();
      // Effective scale from the zoom transform on the canvas content.
      // offsetWidth is unscaled layout size; rect width is scaled.
      const scale =
        wrapper.offsetWidth > 0 ? wrapperRect.width / wrapper.offsetWidth : 1;
      if (!Number.isFinite(scale) || scale <= 0) return;

      const localRect = (el: Element) => {
        const r = el.getBoundingClientRect();
        return {
          right: (r.right - wrapperRect.left) / scale,
          centerY: (r.top + r.height / 2 - wrapperRect.top) / scale,
        };
      };

      // Resolve DOM elements for each edge endpoint
      const resolved: Array<
        ErrorEdgeDef & {
          source: ReturnType<typeof localRect>;
          target: ReturnType<typeof localRect>;
        }
      > = [];
      let maxRight = 0;
      for (const def of defs) {
        const sourceEl = root.querySelector(
          `[data-flow-node-id="${CSS.escape(def.sourceId)}"]`
        );
        const targetEl = root.querySelector(
          `[data-flow-node-id="${CSS.escape(def.targetId)}"]`
        );
        if (!sourceEl || !targetEl) continue;
        const source = localRect(sourceEl);
        const target = localRect(targetEl);
        maxRight = Math.max(maxRight, source.right, target.right);
        resolved.push({ ...def, source, target });
      }

      const measured: MeasuredEdge[] = resolved.map((edge, index) => {
        const railX = maxRight + RAIL_OFFSET + index * RAIL_SPACING;
        const sy = edge.source.centerY;
        const ty = edge.target.centerY;
        const path = [
          `M ${edge.source.right} ${sy}`,
          `H ${railX}`,
          `V ${ty}`,
          `H ${edge.target.right + ARROW_GAP}`,
        ].join(' ');
        return {
          sourceId: edge.sourceId,
          targetId: edge.targetId,
          path,
          labelX: railX,
          labelY: (sy + ty) / 2,
        };
      });

      setEdges(measured);
    };

    measure();

    // Re-measure when canvas content resizes (nodes added/edited/moved)
    const resizeObserver =
      typeof ResizeObserver !== 'undefined'
        ? new ResizeObserver(() => measure())
        : null;
    if (resizeObserver && wrapper.parentElement) {
      resizeObserver.observe(wrapper.parentElement);
    }
    window.addEventListener('resize', measure);

    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [flow]);

  if (edges.length === 0) {
    // Keep the wrapper mounted so measurement works when edges appear
    return (
      <div
        ref={wrapperRef}
        className={clsx('absolute inset-0 pointer-events-none', className)}
        aria-hidden="true"
      />
    );
  }

  return (
    <div
      ref={wrapperRef}
      className={clsx(
        'absolute inset-0 pointer-events-none overflow-visible',
        className
      )}
      aria-hidden="true"
    >
      <svg
        className="absolute inset-0 w-full h-full"
        style={{ overflow: 'visible' }}
      >
        <defs>
          <marker
            id="raisin-flow-error-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" fill="#ef4444" />
          </marker>
        </defs>
        {edges.map((edge) => {
          const active = failedNodeIds?.has(edge.sourceId) ?? false;
          return (
            <path
              key={`${edge.sourceId}->${edge.targetId}`}
              d={edge.path}
              fill="none"
              strokeDasharray="6 4"
              strokeWidth={active ? 2.5 : 1.5}
              className={clsx(
                'transition-colors duration-200',
                active ? 'stroke-red-500' : 'stroke-red-400/70'
              )}
              markerEnd="url(#raisin-flow-error-arrow)"
              data-wf-error-edge="true"
            />
          );
        })}
      </svg>
      {/* Edge label badges (same styling as ErrorEdge) */}
      {edges.map((edge) => {
        const active = failedNodeIds?.has(edge.sourceId) ?? false;
        return (
          <div
            key={`label-${edge.sourceId}->${edge.targetId}`}
            className={clsx(
              'absolute flex items-center gap-1 px-2 py-0.5 rounded-full',
              'text-xs font-medium whitespace-nowrap shadow-sm',
              active
                ? 'bg-red-500 text-white'
                : 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
              'transition-colors duration-200'
            )}
            style={{
              left: edge.labelX,
              top: edge.labelY,
              transform: 'translate(-50%, -50%)',
            }}
          >
            <AlertTriangle className="w-3 h-3" />
            <span>on error</span>
          </div>
        );
      })}
    </div>
  );
}

export default ErrorEdgeOverlay;
