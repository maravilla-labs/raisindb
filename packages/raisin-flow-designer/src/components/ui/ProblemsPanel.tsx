/**
 * Problems Panel Component
 *
 * Displays a list of validation issues (errors, warnings, suggestions)
 * with the ability to click and navigate to the problematic node.
 */

import { clsx } from 'clsx';
import { AlertCircle, AlertTriangle, Info, ChevronDown, ChevronRight } from 'lucide-react';
import { useState, useCallback } from 'react';
import type { ValidationResult, ValidationIssue } from '../../context/FlowDesignerContext';
import { useThemeClasses } from '../../context';

export interface ProblemsPanelProps {
  /** Validation result containing issues */
  validation: ValidationResult;
  /** Callback when a node issue is clicked */
  onNodeClick?: (nodeId: string) => void;
  /** Whether the panel can be collapsed */
  collapsible?: boolean;
  /** Initial collapsed state */
  defaultCollapsed?: boolean;
  /** Custom class name */
  className?: string;
  /** Maximum height (scrollable) */
  maxHeight?: number;
}

export function ProblemsPanel({
  validation,
  onNodeClick,
  collapsible = false,
  defaultCollapsed = false,
  className,
  maxHeight = 300,
}: ProblemsPanelProps) {
  const [isCollapsed, setIsCollapsed] = useState(defaultCollapsed);
  const [expandedSections, setExpandedSections] = useState<Record<string, boolean>>({
    errors: true,
    warnings: true,
    suggestions: false,
  });
  const themeClasses = useThemeClasses();

  const { errors, warnings, suggestions } = validation;
  const totalIssues = errors.length + warnings.length + suggestions.length;

  const toggleSection = useCallback((section: string) => {
    setExpandedSections(prev => ({
      ...prev,
      [section]: !prev[section],
    }));
  }, []);

  const handleIssueClick = useCallback((issue: ValidationIssue) => {
    if (issue.nodeId && onNodeClick) {
      onNodeClick(issue.nodeId);
    }
  }, [onNodeClick]);

  if (totalIssues === 0) {
    return (
      <div className={clsx(
        'flex items-center gap-2 px-3 py-2 rounded-lg',
        'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
        className
      )}>
        <AlertCircle className="w-4 h-4" />
        <span className="text-sm font-medium">No problems found</span>
      </div>
    );
  }

  return (
    <div
      className={clsx(
        'rounded-lg border overflow-hidden',
        themeClasses.stepBg,
        themeClasses.stepBorder,
        className
      )}
    >
      {/* Header */}
      <div
        className={clsx(
          'flex items-center justify-between px-3 py-2',
          'border-b',
          themeClasses.stepBorder,
          collapsible && 'cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800'
        )}
        onClick={collapsible ? () => setIsCollapsed(!isCollapsed) : undefined}
      >
        <div className="flex items-center gap-2">
          {collapsible && (
            isCollapsed ? (
              <ChevronRight className="w-4 h-4" />
            ) : (
              <ChevronDown className="w-4 h-4" />
            )
          )}
          <span className="text-sm font-semibold">Problems</span>
          <span className={clsx(
            'px-1.5 py-0.5 rounded-full text-xs font-medium',
            errors.length > 0
              ? 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
              : 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400'
          )}>
            {totalIssues}
          </span>
        </div>
      </div>

      {/* Content */}
      {!isCollapsed && (
        <div
          className="overflow-y-auto"
          style={{ maxHeight: `${maxHeight}px` }}
        >
          {/* Errors Section */}
          {errors.length > 0 && (
            <IssueSection
              title="Errors"
              icon={AlertCircle}
              issues={errors}
              isExpanded={expandedSections.errors}
              onToggle={() => toggleSection('errors')}
              onIssueClick={handleIssueClick}
              iconClassName="text-red-500"
            />
          )}

          {/* Warnings Section */}
          {warnings.length > 0 && (
            <IssueSection
              title="Warnings"
              icon={AlertTriangle}
              issues={warnings}
              isExpanded={expandedSections.warnings}
              onToggle={() => toggleSection('warnings')}
              onIssueClick={handleIssueClick}
              iconClassName="text-amber-500"
            />
          )}

          {/* Suggestions Section */}
          {suggestions.length > 0 && (
            <IssueSection
              title="Suggestions"
              icon={Info}
              issues={suggestions}
              isExpanded={expandedSections.suggestions}
              onToggle={() => toggleSection('suggestions')}
              onIssueClick={handleIssueClick}
              iconClassName="text-blue-500"
            />
          )}
        </div>
      )}
    </div>
  );
}

interface IssueSectionProps {
  title: string;
  icon: React.ComponentType<{ className?: string }>;
  issues: ValidationIssue[];
  isExpanded: boolean;
  onToggle: () => void;
  onIssueClick: (issue: ValidationIssue) => void;
  iconClassName: string;
}

function IssueSection({
  title,
  icon: Icon,
  issues,
  isExpanded,
  onToggle,
  onIssueClick,
  iconClassName,
}: IssueSectionProps) {
  const themeClasses = useThemeClasses();

  return (
    <div className="border-b last:border-b-0 dark:border-gray-700">
      {/* Section Header */}
      <button
        type="button"
        onClick={onToggle}
        className={clsx(
          'w-full flex items-center gap-2 px-3 py-2 text-left',
          'hover:bg-gray-50 dark:hover:bg-gray-800',
          'transition-colors'
        )}
      >
        {isExpanded ? (
          <ChevronDown className="w-3 h-3 text-gray-500" />
        ) : (
          <ChevronRight className="w-3 h-3 text-gray-500" />
        )}
        <Icon className={clsx('w-4 h-4', iconClassName)} />
        <span className="text-sm font-medium flex-1">{title}</span>
        <span className="text-xs text-gray-500">{issues.length}</span>
      </button>

      {/* Issues List */}
      {isExpanded && (
        <div className="pb-1">
          {issues.map((issue, index) => (
            <button
              key={`${issue.nodeId}-${issue.code}-${index}`}
              type="button"
              onClick={() => onIssueClick(issue)}
              disabled={!issue.nodeId}
              className={clsx(
                'w-full flex items-start gap-2 px-3 py-1.5 text-left',
                'pl-9', // Indent for section icon
                issue.nodeId && 'hover:bg-gray-50 dark:hover:bg-gray-800 cursor-pointer',
                !issue.nodeId && 'cursor-default',
                'transition-colors'
              )}
            >
              <span className={clsx(
                'text-sm flex-1',
                themeClasses.stepText
              )}>
                {issue.message}
              </span>
              {issue.field && (
                <span className="text-xs text-gray-400 font-mono">
                  {issue.field}
                </span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export default ProblemsPanel;
