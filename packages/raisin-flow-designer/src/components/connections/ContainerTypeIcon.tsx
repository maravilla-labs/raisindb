/**
 * Container Type Icon Component
 *
 * SVG icons for AND/OR/Parallel/AI container types.
 * Supports light and dark themes.
 */

import { clsx } from 'clsx';
import type { ContainerType } from '../../types';
import { useThemeClasses } from '../../context';

export interface ContainerTypeIconProps {
  /** Container type */
  containerType: ContainerType;
  /** Number of children */
  childCount?: number;
}

export function ContainerTypeIcon({
  containerType,
  childCount = 0,
}: ContainerTypeIconProps) {
  const themeClasses = useThemeClasses();

  // Only show if more than 1 child (matches Svelte behavior)
  if (childCount <= 1) {
    return null;
  }

  // Theme-aware colors - use transparent light blues for dark mode
  const strokeColor = themeClasses.isDark ? 'text-blue-200/50' : 'text-blue-200';
  const fillColor = themeClasses.isDark ? 'fill-blue-50/10' : 'fill-blue-50';
  const shapeFill = themeClasses.isDark ? 'fill-blue-50/10' : 'fill-white';
  const eraseLine = themeClasses.isDark ? 'stroke-blue-50/10' : 'stroke-blue-50';

  if (containerType === 'and') {
    return (
      <svg
        width="30"
        height="30"
        viewBox="0 0 24 24"
        className={clsx(
          'absolute z-20 -bottom-[6px] transform -left-[14.5px] stroke-current',
          strokeColor,
          fillColor
        )}
      >
        {/* Filled Triangle */}
        <polygon
          points="12,3 3,21 21,21"
          strokeWidth="1"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        {/* "Erase" the Bottom Line */}
        <line x1="3" y1="21" x2="21" y2="21" strokeWidth="2" className={eraseLine} />
        {childCount > 2 && (
          <line x1="12" y1="3" x2="12" y2="23" strokeWidth="1" />
        )}
      </svg>
    );
  }

  return (
    <svg
      width="30"
      height="30"
      viewBox="0 0 24 24"
      className={clsx(
        'absolute z-20 -bottom-[15px] transform -left-[14.5px] stroke-current',
        strokeColor,
        shapeFill
      )}
    >
      {containerType === 'parallel' && (
        <>
          {/* Parallel (Two Bars in a Rectangle) */}
          <rect x="3" y="5" width="18" height="14" rx="2" ry="2" strokeWidth="2" />
          <line x1="8" y1="5" x2="8" y2="19" strokeWidth="2" />
          <line x1="16" y1="5" x2="16" y2="19" strokeWidth="2" />
        </>
      )}
      {containerType === 'or' && (
        /* OR (Decision Diamond) */
        <polygon
          points="12,3 21,12 12,21 3,12"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      )}
      {containerType === 'ai_sequence' && (
        /* AI Sequence (Diamond + Yellow fill) */
        <polygon
          points="12,3 21,12 12,21 3,12"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          fill="yellow"
        />
      )}
    </svg>
  );
}
