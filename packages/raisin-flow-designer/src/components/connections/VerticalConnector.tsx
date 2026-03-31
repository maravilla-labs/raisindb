/**
 * Vertical Connector Component
 *
 * Visible vertical line connecting nodes.
 * Supports light and dark themes.
 */

import { clsx } from 'clsx';
import { useThemeClasses } from '../../context';

export interface VerticalConnectorProps {
  /** Height of the connector in pixels (default 40px) */
  height?: number;
  /** Custom class name */
  className?: string;
}

export function VerticalConnector({
  height = 40,
  className,
}: VerticalConnectorProps) {
  const themeClasses = useThemeClasses();

  return (
    <div
      className={clsx('w-[1px]', themeClasses.connectorBg, className)}
      style={{ height: `${height}px` }}
      data-wf-vertical-line="true"
    />
  );
}
