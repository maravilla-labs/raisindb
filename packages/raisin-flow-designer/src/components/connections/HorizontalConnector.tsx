/**
 * Horizontal Connector Component
 *
 * Visible horizontal line for connecting parallel branches.
 * Matches Svelte: h-[1px] bg-blue-200
 */

import { clsx } from 'clsx';

export interface HorizontalConnectorProps {
  /** Width of the connector (default: 100%) */
  width?: number | string;
  /** Custom class name */
  className?: string;
}

export function HorizontalConnector({
  width = '100%',
  className,
}: HorizontalConnectorProps) {
  return (
    <div
      className={clsx('h-[1px] bg-blue-200', className)}
      style={{
        width: typeof width === 'number' ? `${width}px` : width,
        margin: '0 auto'
      }}
      data-wf-horizontal-line="true"
    />
  );
}
