/**
 * Drop Indicator Component
 *
 * 2px blue line with circle endpoints showing drop position.
 */

import { clsx } from 'clsx';
import type { DropIndicatorState } from '../../types';

export interface DropIndicatorProps extends DropIndicatorState {
  /** Custom class name */
  className?: string;
}

export function DropIndicator({
  visible,
  orientation,
  position,
  size,
  insertPosition,
  className,
}: DropIndicatorProps) {
  if (!visible || insertPosition === 'inside') return null;

  const isVertical = orientation === 'vertical';

  return (
    <div
      className={clsx(
        'fixed pointer-events-none z-50',
        isVertical ? 'w-[2px]' : 'h-[2px]',
        'bg-blue-400',
        className
      )}
      style={{
        left: position.x,
        top: position.y,
        [isVertical ? 'height' : 'width']: size,
      }}
    >
      {/* Top/Left endpoint circle */}
      <div
        className={clsx(
          'absolute w-3 h-3 rounded-full',
          'border-2 border-blue-400 bg-white',
          isVertical
            ? '-left-[5px] -top-[6px]'
            : '-top-[5px] -left-[6px]'
        )}
      />
      {/* Bottom/Right endpoint circle */}
      <div
        className={clsx(
          'absolute w-3 h-3 rounded-full',
          'border-2 border-blue-400 bg-white',
          isVertical
            ? '-left-[5px] -bottom-[6px]'
            : '-top-[5px] -right-[6px]'
        )}
      />
    </div>
  );
}
