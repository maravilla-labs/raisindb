/**
 * Trigger Node Component
 *
 * Displays triggers before the start node in FlowDesigner.
 * Shows trigger type icon and name with visual distinction by type.
 * Supports light and dark themes.
 */

import { clsx } from 'clsx';
import { Globe, Clock, Zap, FileCode } from 'lucide-react';

export type TriggerType = 'node_event' | 'schedule' | 'http';

export interface TriggerNodeProps {
  /** Trigger ID */
  id: string;
  /** Trigger name */
  name: string;
  /** Trigger type */
  triggerType: TriggerType;
  /** Whether trigger is enabled */
  enabled?: boolean;
  /** Whether node is selected */
  selected?: boolean;
  /** Click handler */
  onClick?: () => void;
  /** Webhook ID (for HTTP triggers) */
  webhookId?: string;
  /** Custom class name */
  className?: string;
}

const TRIGGER_ICONS: Record<TriggerType, React.ReactNode> = {
  node_event: <FileCode className="w-4 h-4" />,
  schedule: <Clock className="w-4 h-4" />,
  http: <Globe className="w-4 h-4" />,
};

const TRIGGER_LABELS: Record<TriggerType, string> = {
  node_event: 'Event',
  schedule: 'Schedule',
  http: 'HTTP',
};

export function TriggerNode({
  id,
  name,
  triggerType,
  enabled = true,
  selected = false,
  onClick,
  webhookId,
  className,
}: TriggerNodeProps) {
  // Type-specific styling
  const typeStyles: Record<TriggerType, string> = {
    node_event: 'bg-purple-500/20 border-purple-500/50 text-purple-400 hover:border-purple-400',
    schedule: 'bg-orange-500/20 border-orange-500/50 text-orange-400 hover:border-orange-400',
    http: 'bg-green-500/20 border-green-500/50 text-green-400 hover:border-green-400',
  };

  return (
    <div
      className={clsx('flex flex-col items-center gap-1', className)}
      data-trigger-id={id}
      data-trigger-type={triggerType}
    >
      <button
        onClick={onClick}
        className={clsx(
          'px-3 py-2 rounded-lg border cursor-pointer transition-all duration-200',
          'flex items-center gap-2 min-w-[100px] justify-center',
          typeStyles[triggerType],
          !enabled && 'opacity-50 grayscale',
          selected && 'ring-2 ring-blue-500 ring-offset-1 ring-offset-white dark:ring-offset-gray-900'
        )}
        title={webhookId ? `Webhook ID: ${webhookId}` : undefined}
      >
        {TRIGGER_ICONS[triggerType]}
        <span className="text-sm font-medium truncate max-w-[120px]">{name}</span>
      </button>
      <span
        className={clsx(
          'text-xs',
          enabled ? 'text-gray-500 dark:text-gray-400' : 'text-gray-400 dark:text-gray-500'
        )}
      >
        {TRIGGER_LABELS[triggerType]}
        {!enabled && ' (disabled)'}
      </span>
    </div>
  );
}

/**
 * Add Trigger Button Component
 *
 * Button to add a new trigger to the flow.
 */
export interface AddTriggerButtonProps {
  /** Click handler */
  onClick?: () => void;
  /** Whether button is disabled */
  disabled?: boolean;
  /** Custom class name */
  className?: string;
}

export function AddTriggerButton({
  onClick,
  disabled = false,
  className,
}: AddTriggerButtonProps) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={clsx(
        'px-3 py-2 rounded-lg border-2 border-dashed',
        'flex items-center gap-2 min-w-[100px] justify-center',
        'transition-all duration-200',
        'text-gray-400 border-gray-300 dark:text-gray-500 dark:border-gray-600',
        !disabled && 'hover:text-blue-500 hover:border-blue-500 dark:hover:text-blue-400 dark:hover:border-blue-400',
        disabled && 'opacity-50 cursor-not-allowed',
        className
      )}
    >
      <Zap className="w-4 h-4" />
      <span className="text-sm font-medium">Add Trigger</span>
    </button>
  );
}

/**
 * Trigger Info type for passing trigger data
 */
export interface TriggerInfo {
  /** Trigger node ID */
  id: string;
  /** Trigger name */
  name: string;
  /** Type of trigger */
  triggerType: TriggerType;
  /** Whether trigger is enabled */
  enabled: boolean;
  /** Webhook ID (for HTTP triggers) */
  webhookId?: string;
  /** Trigger configuration summary */
  configSummary?: string;
}
