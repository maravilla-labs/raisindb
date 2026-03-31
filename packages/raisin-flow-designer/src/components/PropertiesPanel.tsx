/**
 * Properties Panel Component
 *
 * Displays and edits properties for the currently selected flow node.
 * Shows different editors based on node type (step, container, etc.).
 */

import { clsx } from 'clsx';
import { X, Settings, Box, GitBranch } from 'lucide-react';
import { useFlowDesignerContext } from '../context/FlowDesignerContext';
import { useThemeClasses } from '../context';
import { isFlowStep, isFlowContainer } from '../utils';
import { StepPropertiesEditor } from './properties/StepPropertiesEditor';
import { ErrorHandlingEditor } from './properties/ErrorHandlingEditor';
import type { FlowStep, FlowContainer } from '../types';

export interface PropertiesPanelProps {
  /** Whether the panel is open */
  isOpen?: boolean;
  /** Callback when panel is closed */
  onClose?: () => void;
  /** Custom class name */
  className?: string;
  /** Panel width in pixels */
  width?: number;
}

export function PropertiesPanel({
  isOpen = true,
  onClose,
  className,
  width = 320,
}: PropertiesPanelProps) {
  const { selectedNodes, selectedNodeIds } = useFlowDesignerContext();
  const themeClasses = useThemeClasses();

  if (!isOpen) return null;

  const selectedNode = selectedNodes[0];
  const hasSelection = selectedNodeIds.length > 0;
  const hasMultipleSelection = selectedNodeIds.length > 1;

  return (
    <div
      className={clsx(
        'flex flex-col h-full border-l overflow-hidden',
        themeClasses.stepBg,
        themeClasses.stepBorder,
        className
      )}
      style={{ width: `${width}px`, minWidth: `${width}px` }}
    >
      {/* Header */}
      <div
        className={clsx(
          'flex items-center justify-between px-4 py-3 border-b shrink-0',
          themeClasses.stepBorder
        )}
      >
        <div className="flex items-center gap-2">
          <Settings className="w-4 h-4 text-gray-500" />
          <span className={clsx('font-semibold', themeClasses.stepText)}>
            Properties
          </span>
        </div>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            className={clsx(
              'p-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800',
              'transition-colors'
            )}
          >
            <X className="w-4 h-4 text-gray-500" />
          </button>
        )}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto">
        {!hasSelection ? (
          <EmptyState message="Select a node to view its properties" />
        ) : hasMultipleSelection ? (
          <EmptyState message={`${selectedNodeIds.length} nodes selected. Select a single node to edit properties.`} />
        ) : selectedNode && isFlowStep(selectedNode) ? (
          <StepPropertiesContent step={selectedNode as FlowStep} />
        ) : selectedNode && isFlowContainer(selectedNode) ? (
          <ContainerPropertiesContent container={selectedNode as FlowContainer} />
        ) : (
          <EmptyState message="Unknown node type" />
        )}
      </div>
    </div>
  );
}

function EmptyState({ message }: { message: string }) {
  const themeClasses = useThemeClasses();

  return (
    <div className="flex flex-col items-center justify-center h-full p-8 text-center">
      <Box className="w-12 h-12 text-gray-300 dark:text-gray-600 mb-4" />
      <p className={clsx('text-sm', themeClasses.stepTextMuted)}>
        {message}
      </p>
    </div>
  );
}

function StepPropertiesContent({ step }: { step: FlowStep }) {
  const themeClasses = useThemeClasses();

  return (
    <div className="p-4 space-y-6">
      {/* Step Type Badge */}
      <div className="flex items-center gap-2">
        <span className={clsx(
          'px-2 py-1 rounded text-xs font-medium',
          step.properties.step_type === 'ai_agent'
            ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
            : step.properties.step_type === 'human_task'
            ? 'bg-orange-100 text-orange-700 dark:bg-orange-900/30 dark:text-orange-400'
            : 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400'
        )}>
          {step.properties.step_type === 'ai_agent' ? 'AI Agent' :
           step.properties.step_type === 'human_task' ? 'Human Task' :
           'Function Step'}
        </span>
        {step.properties.disabled && (
          <span className="px-2 py-1 rounded text-xs font-medium bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400">
            Disabled
          </span>
        )}
      </div>

      {/* Basic Properties */}
      <StepPropertiesEditor step={step} />

      {/* Error Handling */}
      <div className="pt-4 border-t border-gray-200 dark:border-gray-700">
        <h3 className={clsx(
          'text-sm font-semibold mb-4 flex items-center gap-2',
          themeClasses.stepText
        )}>
          <GitBranch className="w-4 h-4" />
          Error Handling
        </h3>
        <ErrorHandlingEditor step={step} />
      </div>
    </div>
  );
}

function ContainerPropertiesContent({ container }: { container: FlowContainer }) {
  const themeClasses = useThemeClasses();

  return (
    <div className="p-4 space-y-4">
      {/* Container Type */}
      <div>
        <label className={clsx('block text-xs font-medium mb-1', themeClasses.stepTextMuted)}>
          Container Type
        </label>
        <span className={clsx(
          'px-2 py-1 rounded text-xs font-medium',
          container.container_type === 'parallel'
            ? 'bg-indigo-100 text-indigo-700 dark:bg-indigo-900/30 dark:text-indigo-400'
            : container.container_type === 'ai_sequence'
            ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
            : 'bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-400'
        )}>
          {container.container_type === 'parallel' ? 'Parallel' :
           container.container_type === 'ai_sequence' ? 'AI Sequence' :
           container.container_type === 'and' ? 'AND' :
           container.container_type === 'or' ? 'OR' :
           container.container_type}
        </span>
      </div>

      {/* Children Count */}
      <div>
        <label className={clsx('block text-xs font-medium mb-1', themeClasses.stepTextMuted)}>
          Children
        </label>
        <p className={clsx('text-sm', themeClasses.stepText)}>
          {container.children.length} node{container.children.length !== 1 ? 's' : ''}
        </p>
      </div>

      {/* Container Rules */}
      {container.rules && container.rules.length > 0 && (
        <div>
          <label className={clsx('block text-xs font-medium mb-1', themeClasses.stepTextMuted)}>
            Rules
          </label>
          <ul className="text-sm space-y-1">
            {container.rules.map((rule, i) => (
              <li key={i} className={themeClasses.stepText}>
                {rule.condition} → {rule.next_step}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* AI Config */}
      {container.container_type === 'ai_sequence' && container.ai_config && (
        <div>
          <label className={clsx('block text-xs font-medium mb-1', themeClasses.stepTextMuted)}>
            AI Configuration
          </label>
          <pre className={clsx(
            'text-xs p-2 rounded bg-gray-100 dark:bg-gray-800 overflow-x-auto',
            themeClasses.stepText
          )}>
            {JSON.stringify(container.ai_config, null, 2)}
          </pre>
        </div>
      )}
    </div>
  );
}

export default PropertiesPanel;
