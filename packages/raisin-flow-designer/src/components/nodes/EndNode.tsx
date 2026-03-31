/**
 * End Node Component
 *
 * Exit point node for the flow - green circle with "end" text.
 * Supports light and dark themes.
 */

import { clsx } from 'clsx';
import { useThemeClasses } from '../../context';

export interface EndNodeProps {
  /** Whether node is selected */
  selected?: boolean;
  /** Click handler */
  onClick?: () => void;
  /** Custom class name */
  className?: string;
}

export function EndNode({
  selected = false,
  onClick,
  className,
}: EndNodeProps) {
  const themeClasses = useThemeClasses();

  return (
    <div
      title="Workflow ends here"
      className={clsx(
        'size-[100px] rounded-full flex items-center justify-center',
        themeClasses.endBg,
        themeClasses.endBgHover,
        'border',
        themeClasses.endBorder,
        themeClasses.endText,
        'font-medium',
        'cursor-pointer transition-all duration-200',
        'select-none',
        selected && 'outline-2 outline outline-green-500',
        className
      )}
      onClick={onClick}
    >
      end
    </div>
  );
}
