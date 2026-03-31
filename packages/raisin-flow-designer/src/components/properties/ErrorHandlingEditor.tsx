/**
 * Error Handling Editor Component
 *
 * Form for editing error handling properties of a FlowStep including
 * error edges, compensation references, and error behavior settings.
 */

import { useCallback } from 'react';
import { clsx } from 'clsx';
import { AlertTriangle, RotateCcw, GitBranch, Shield } from 'lucide-react';
import { useFlowDesignerContext } from '../../context/FlowDesignerContext';
import { useThemeClasses } from '../../context';
import { UpdateStepCommand } from '../../commands';
import type { FlowStep, FlowStepProperties, StepErrorBehavior, RaisinReference } from '../../types';

export interface ErrorHandlingEditorProps {
  /** The step to edit */
  step: FlowStep;
  /** Custom class name */
  className?: string;
}

export function ErrorHandlingEditor({
  step,
  className,
}: ErrorHandlingEditorProps) {
  const { commandContext, executeCommand, flow } = useFlowDesignerContext();
  const themeClasses = useThemeClasses();

  const updateProperty = useCallback(
    <K extends keyof FlowStepProperties>(key: K, value: FlowStepProperties[K]) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId: step.id,
        updates: { [key]: value } as Partial<FlowStepProperties>,
      });
      executeCommand(command);
    },
    [commandContext, executeCommand, step.id]
  );

  const updateOnError = useCallback(
    (value: StepErrorBehavior) => {
      const command = new UpdateStepCommand(commandContext, {
        nodeId: step.id,
        updates: { on_error: value },
      });
      executeCommand(command);
    },
    [commandContext, executeCommand, step.id]
  );

  // Collect all node IDs for error edge dropdown
  const allNodeIds = collectNodeIds(flow.nodes);
  const errorEdgeOptions = allNodeIds.filter(id => id !== step.id);

  // Get the current error edge from step level
  const currentErrorEdge = step.error_edge || step.properties.error_edge;

  return (
    <div className={clsx('space-y-4', className)}>
      {/* Error Behavior */}
      <FieldGroup label="On Error" icon={<AlertTriangle className="w-4 h-4" />}>
        <select
          value={step.on_error || 'stop'}
          onChange={(e) => updateOnError(e.target.value as StepErrorBehavior)}
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        >
          <option value="stop">Stop workflow</option>
          <option value="skip">Skip this step</option>
          <option value="continue">Continue to next step</option>
        </select>
        <p className={clsx('text-xs mt-1', themeClasses.stepTextFaint)}>
          What happens when this step fails
        </p>
      </FieldGroup>

      {/* Error Edge */}
      <FieldGroup label="Error Handler Node (optional)" icon={<GitBranch className="w-4 h-4" />}>
        <select
          value={currentErrorEdge || ''}
          onChange={(e) => updateProperty('error_edge', e.target.value || undefined)}
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        >
          <option value="">None</option>
          {errorEdgeOptions.map(nodeId => (
            <option key={nodeId} value={nodeId}>{nodeId}</option>
          ))}
        </select>
        <p className={clsx('text-xs mt-1', themeClasses.stepTextFaint)}>
          Node to execute when this step fails (takes precedence over "On Error")
        </p>
      </FieldGroup>

      {/* Compensation Reference */}
      <FieldGroup label="Compensation Function" icon={<RotateCcw className="w-4 h-4" />}>
        <input
          type="text"
          value={getRefDisplayValue(step.properties.compensation_ref)}
          onChange={(e) => updateProperty('compensation_ref', createRef(e.target.value))}
          placeholder="e.g., /functions/rollback-action"
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm font-mono',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        />
        <p className={clsx('text-xs mt-1', themeClasses.stepTextFaint)}>
          Function to call for saga rollback if later steps fail
        </p>
      </FieldGroup>

      {/* Continue on Fail Toggle */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <AlertTriangle className="w-4 h-4 text-amber-500" />
          <span className={clsx('text-sm font-medium', themeClasses.stepText)}>
            Continue on Failure
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={step.properties.continue_on_fail || false}
          onClick={() => updateProperty('continue_on_fail', !step.properties.continue_on_fail)}
          className={clsx(
            'relative inline-flex h-6 w-11 items-center rounded-full transition-colors',
            step.properties.continue_on_fail
              ? 'bg-amber-500'
              : 'bg-gray-300 dark:bg-gray-600'
          )}
        >
          <span
            className={clsx(
              'inline-block h-4 w-4 transform rounded-full bg-white transition-transform',
              step.properties.continue_on_fail ? 'translate-x-6' : 'translate-x-1'
            )}
          />
        </button>
      </div>
      <p className={clsx('text-xs', themeClasses.stepTextFaint, '-mt-2')}>
        Continue workflow execution even if this step fails
      </p>

      {/* Isolated Branch Toggle (for AI steps) */}
      {step.properties.step_type === 'ai_agent' && (
        <>
          <div className="flex items-center justify-between pt-2">
            <div className="flex items-center gap-2">
              <Shield className="w-4 h-4 text-blue-500" />
              <span className={clsx('text-sm font-medium', themeClasses.stepText)}>
                Isolated Branch
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={step.properties.isolated_branch || false}
              onClick={() => updateProperty('isolated_branch', !step.properties.isolated_branch)}
              className={clsx(
                'relative inline-flex h-6 w-11 items-center rounded-full transition-colors',
                step.properties.isolated_branch
                  ? 'bg-blue-500'
                  : 'bg-gray-300 dark:bg-gray-600'
              )}
            >
              <span
                className={clsx(
                  'inline-block h-4 w-4 transform rounded-full bg-white transition-transform',
                  step.properties.isolated_branch ? 'translate-x-6' : 'translate-x-1'
                )}
              />
            </button>
          </div>
          <p className={clsx('text-xs', themeClasses.stepTextFaint, '-mt-2')}>
            Execute AI agent in a git-like isolated branch for safety
          </p>
        </>
      )}
    </div>
  );
}

interface FieldGroupProps {
  label: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}

function FieldGroup({ label, icon, children }: FieldGroupProps) {
  const themeClasses = useThemeClasses();

  return (
    <div>
      <label className={clsx(
        'block text-xs font-medium mb-1.5 flex items-center gap-1.5',
        themeClasses.stepTextMuted
      )}>
        {icon}
        {label}
      </label>
      {children}
    </div>
  );
}

/**
 * Recursively collect all node IDs from the flow
 */
function collectNodeIds(nodes: import('../../types').FlowNode[]): string[] {
  const ids: string[] = [];
  for (const node of nodes) {
    ids.push(node.id);
    if (node.node_type === 'raisin:FlowContainer' && 'children' in node) {
      ids.push(...collectNodeIds(node.children));
    }
  }
  return ids;
}

/**
 * Get display value from a RaisinReference
 */
function getRefDisplayValue(ref: RaisinReference | undefined): string {
  if (!ref) return '';
  return ref['raisin:path'] || ref['raisin:ref'] || '';
}

/**
 * Create a RaisinReference from a string value
 */
function createRef(value: string): RaisinReference | undefined {
  if (!value) return undefined;
  return {
    'raisin:ref': value,
    'raisin:workspace': 'default',
  };
}

export default ErrorHandlingEditor;
