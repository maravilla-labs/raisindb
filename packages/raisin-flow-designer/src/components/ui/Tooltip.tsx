/**
 * Tooltip Component
 *
 * CSS-only tooltip wrapper for hover hints.
 * Uses group-hover pattern for pure CSS tooltips without JavaScript.
 */

import { clsx } from 'clsx';

export type TooltipPosition = 'top' | 'bottom' | 'left' | 'right';

export interface TooltipProps {
  /** Content to show in tooltip */
  content: React.ReactNode;
  /** Position of tooltip relative to children */
  position?: TooltipPosition;
  /** The element to wrap with tooltip */
  children: React.ReactNode;
  /** Additional class name for wrapper */
  className?: string;
  /** Delay before showing tooltip (ms) - CSS transition */
  delay?: number;
}

const positionClasses: Record<TooltipPosition, string> = {
  top: 'bottom-full left-1/2 -translate-x-1/2 mb-2',
  bottom: 'top-full left-1/2 -translate-x-1/2 mt-2',
  left: 'right-full top-1/2 -translate-y-1/2 mr-2',
  right: 'left-full top-1/2 -translate-y-1/2 ml-2',
};

const arrowClasses: Record<TooltipPosition, string> = {
  top: 'top-full left-1/2 -translate-x-1/2 border-t-gray-900 border-x-transparent border-b-transparent',
  bottom: 'bottom-full left-1/2 -translate-x-1/2 border-b-gray-900 border-x-transparent border-t-transparent',
  left: 'left-full top-1/2 -translate-y-1/2 border-l-gray-900 border-y-transparent border-r-transparent',
  right: 'right-full top-1/2 -translate-y-1/2 border-r-gray-900 border-y-transparent border-l-transparent',
};

export function Tooltip({
  content,
  position = 'top',
  children,
  className,
  delay = 200,
}: TooltipProps) {
  return (
    <div className={clsx('relative group inline-flex', className)}>
      {children}
      <div
        className={clsx(
          'absolute z-50 px-2 py-1 text-xs font-medium rounded',
          'bg-gray-900 text-white whitespace-nowrap',
          'opacity-0 invisible group-hover:opacity-100 group-hover:visible',
          'transition-all pointer-events-none',
          'shadow-lg',
          positionClasses[position]
        )}
        style={{ transitionDelay: `${delay}ms` }}
        role="tooltip"
      >
        {content}
        {/* Arrow */}
        <div
          className={clsx(
            'absolute w-0 h-0 border-4',
            arrowClasses[position]
          )}
        />
      </div>
    </div>
  );
}
