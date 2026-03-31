/**
 * Error Edge Component
 *
 * Visually distinct connector for error handling flow paths.
 * Shows a dashed red line with an error icon to indicate
 * the path taken when a step fails.
 */

import { clsx } from 'clsx';
import { AlertTriangle } from 'lucide-react';

export interface ErrorEdgeProps {
  /** Height of the connector in pixels (default 40px) */
  height?: number;
  /** Whether this edge is currently active/highlighted */
  isActive?: boolean;
  /** Custom class name */
  className?: string;
  /** Label to display on the edge */
  label?: string;
}

export function ErrorEdge({
  height = 40,
  isActive = false,
  className,
  label = 'on error',
}: ErrorEdgeProps) {
  return (
    <div
      className={clsx(
        'relative flex items-center justify-center',
        className
      )}
      style={{ height: `${height}px`, minWidth: '100px' }}
    >
      {/* Dashed error line */}
      <div
        className={clsx(
          'w-[2px] border-l-2 border-dashed',
          isActive ? 'border-red-500' : 'border-red-400/70',
          'transition-colors duration-200'
        )}
        style={{ height: `${height}px` }}
        data-wf-error-edge="true"
      />

      {/* Error indicator badge */}
      <div
        className={clsx(
          'absolute flex items-center gap-1 px-2 py-0.5 rounded-full',
          'text-xs font-medium',
          isActive
            ? 'bg-red-500 text-white'
            : 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
          'transition-colors duration-200',
          'shadow-sm'
        )}
      >
        <AlertTriangle className="w-3 h-3" />
        <span>{label}</span>
      </div>
    </div>
  );
}

export default ErrorEdge;
