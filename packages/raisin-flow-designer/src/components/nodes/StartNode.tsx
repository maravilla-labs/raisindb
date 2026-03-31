/**
 * Start Node Component
 *
 * Entry point node for the flow - blue circle with "start" text.
 * Supports light and dark themes.
 */

import { clsx } from 'clsx';
import { useThemeClasses } from '../../context';

export interface StartNodeProps {
  /** Optional trigger type (for future icon display) */
  triggerType?: 'node_event' | 'schedule' | 'http';
  /** Whether node is selected */
  selected?: boolean;
  /** Click handler */
  onClick?: () => void;
  /** Custom class name */
  className?: string;
}

export function StartNode({
  triggerType: _triggerType,
  selected = false,
  onClick,
  className,
}: StartNodeProps) {
  const themeClasses = useThemeClasses();

  return (
    <div
      title="Workflow starts here"
      className={clsx(
        'size-[100px] rounded-full flex items-center justify-center',
        themeClasses.startBg,
        themeClasses.startBgHover,
        'border',
        themeClasses.startBorder,
        themeClasses.startText,
        'font-medium',
        'cursor-pointer transition-all duration-200',
        'select-none',
        selected && 'outline-2 outline outline-blue-500',
        className
      )}
      onClick={onClick}
    >
      start
    </div>
  );
}
