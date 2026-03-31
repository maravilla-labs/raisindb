/**
 * Connector With Button Component
 *
 * Vertical connector with a plus button in the middle for adding steps.
 * Supports light and dark themes.
 */

import { useState } from 'react';
import { clsx } from 'clsx';
import { Plus } from 'lucide-react';
import { useThemeClasses } from '../../context';

export interface ConnectorWithButtonProps {
  /** Height of the connector in pixels */
  height?: number;
  /** Handler when plus button is clicked */
  onAdd?: () => void;
  /** Custom class name */
  className?: string;
}

export function ConnectorWithButton({
  height = 40,
  onAdd,
  className,
}: ConnectorWithButtonProps) {
  const [isHovered, setIsHovered] = useState(false);
  const themeClasses = useThemeClasses();

  return (
    <div
      className={clsx('relative w-[300px] flex items-center justify-center', className)}
      style={{ height: `${height}px` }}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      {/* Drop zone overlay */}
      <div className="absolute top-0 w-full z-20 min-h-10 bg-transparent h-full left-0" />

      {/* Vertical line */}
      <div
        className={clsx('w-[1px]', themeClasses.connectorBg)}
        style={{ height: `${height}px` }}
        data-wf-vertical-line="true"
      />

      {/* Plus button - shown on hover */}
      {isHovered && onAdd && (
        <button
          title="Click to add a step"
          onClick={(e) => {
            e.stopPropagation();
            onAdd?.();
          }}
          className={clsx(
            'absolute w-6 h-6 rounded-full flex items-center justify-center z-30',
            'transition-all duration-200',
            'bg-blue-500 text-white shadow-md hover:bg-blue-600'
          )}
        >
          <Plus className="w-4 h-4" />
        </button>
      )}
    </div>
  );
}
