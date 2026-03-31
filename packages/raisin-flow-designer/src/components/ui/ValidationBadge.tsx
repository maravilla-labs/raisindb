/**
 * Validation Badge Component
 *
 * Displays a compact badge showing validation status with error/warning counts.
 * Click to expand and view detailed issues.
 */

import { clsx } from 'clsx';
import { AlertCircle, AlertTriangle, CheckCircle, Info } from 'lucide-react';
import type { ValidationResult, ValidationIssue } from '../../context/FlowDesignerContext';

export interface ValidationBadgeProps {
  /** Validation result to display */
  validation: ValidationResult;
  /** Whether the badge is clickable */
  onClick?: () => void;
  /** Size variant */
  size?: 'sm' | 'md' | 'lg';
  /** Show detailed counts */
  showCounts?: boolean;
  /** Custom class name */
  className?: string;
}

export function ValidationBadge({
  validation,
  onClick,
  size = 'md',
  showCounts = true,
  className,
}: ValidationBadgeProps) {
  const { errors, warnings } = validation;

  // Determine overall status
  const status = errors.length > 0 ? 'error' : warnings.length > 0 ? 'warning' : 'valid';

  // Size classes
  const sizeClasses = {
    sm: 'text-xs px-1.5 py-0.5 gap-1',
    md: 'text-sm px-2 py-1 gap-1.5',
    lg: 'text-base px-3 py-1.5 gap-2',
  };

  const iconSizes = {
    sm: 'w-3 h-3',
    md: 'w-4 h-4',
    lg: 'w-5 h-5',
  };

  // Status colors
  const statusClasses = {
    error: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
    warning: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400',
    valid: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
  };

  // Status icon
  const StatusIcon = status === 'error' ? AlertCircle :
                     status === 'warning' ? AlertTriangle :
                     CheckCircle;

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      className={clsx(
        'inline-flex items-center rounded-full font-medium transition-colors',
        sizeClasses[size],
        statusClasses[status],
        onClick && 'cursor-pointer hover:opacity-80',
        !onClick && 'cursor-default',
        className
      )}
      title={
        status === 'valid'
          ? 'Workflow is valid'
          : `${errors.length} error(s), ${warnings.length} warning(s)`
      }
    >
      <StatusIcon className={iconSizes[size]} />

      {showCounts && (
        <span>
          {status === 'valid' ? (
            'Valid'
          ) : (
            <>
              {errors.length > 0 && (
                <span className="text-red-600 dark:text-red-400">
                  {errors.length} error{errors.length !== 1 ? 's' : ''}
                </span>
              )}
              {errors.length > 0 && warnings.length > 0 && ', '}
              {warnings.length > 0 && (
                <span className="text-amber-600 dark:text-amber-400">
                  {warnings.length} warning{warnings.length !== 1 ? 's' : ''}
                </span>
              )}
            </>
          )}
        </span>
      )}
    </button>
  );
}

export interface ValidationIssueBadgeProps {
  /** Single validation issue to display */
  issue: ValidationIssue;
  /** Size variant */
  size?: 'sm' | 'md';
  /** Custom class name */
  className?: string;
}

/**
 * Inline badge for a single validation issue
 */
export function ValidationIssueBadge({
  issue,
  size = 'sm',
  className,
}: ValidationIssueBadgeProps) {
  const { severity, message } = issue;

  const sizeClasses = {
    sm: 'text-xs px-1.5 py-0.5 gap-1',
    md: 'text-sm px-2 py-1 gap-1.5',
  };

  const iconSizes = {
    sm: 'w-3 h-3',
    md: 'w-4 h-4',
  };

  const severityClasses = {
    error: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
    warning: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400',
    suggestion: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
  };

  const Icon = severity === 'error' ? AlertCircle :
               severity === 'warning' ? AlertTriangle :
               Info;

  return (
    <span
      className={clsx(
        'inline-flex items-center rounded font-medium',
        sizeClasses[size],
        severityClasses[severity],
        className
      )}
      title={message}
    >
      <Icon className={iconSizes[size]} />
      <span className="truncate max-w-[200px]">{message}</span>
    </span>
  );
}

export default ValidationBadge;
