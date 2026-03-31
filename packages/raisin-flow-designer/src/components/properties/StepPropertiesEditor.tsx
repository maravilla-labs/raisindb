/**
 * Step Properties Editor Component
 *
 * Form for editing FlowStep properties including action name,
 * function references, conditions, and step-type specific fields.
 */

import { useCallback } from 'react';
import { clsx } from 'clsx';
import { Code, User, Bot, Clock, RotateCcw } from 'lucide-react';
import { useFlowDesignerContext } from '../../context/FlowDesignerContext';
import { useThemeClasses } from '../../context';
import { UpdateStepCommand } from '../../commands';
import type { FlowStep, FlowStepProperties, RetryStrategy, RaisinReference } from '../../types';

export interface StepPropertiesEditorProps {
  /** The step to edit */
  step: FlowStep;
  /** Custom class name */
  className?: string;
}

export function StepPropertiesEditor({
  step,
  className,
}: StepPropertiesEditorProps) {
  const { commandContext, executeCommand } = useFlowDesignerContext();
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

  return (
    <div className={clsx('space-y-4', className)}>
      {/* Action Name */}
      <FieldGroup label="Action Name" icon={<Code className="w-4 h-4" />}>
        <input
          type="text"
          value={step.properties.action || ''}
          onChange={(e) => updateProperty('action', e.target.value)}
          placeholder="Enter action name"
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        />
      </FieldGroup>

      {/* Function Reference (for default step type) */}
      {step.properties.step_type !== 'ai_agent' && step.properties.step_type !== 'human_task' && (
        <FieldGroup label="Function Reference" icon={<Code className="w-4 h-4" />}>
          <input
            type="text"
            value={getRefDisplayValue(step.properties.function_ref)}
            onChange={(e) => updateProperty('function_ref', createRef(e.target.value))}
            placeholder="e.g., /functions/my-function"
            className={clsx(
              'w-full px-3 py-2 rounded-md border text-sm font-mono',
              themeClasses.stepBg,
              themeClasses.stepBorder,
              themeClasses.stepText,
              'focus:outline-none focus:ring-2 focus:ring-blue-500'
            )}
          />
          <p className={clsx('text-xs mt-1', themeClasses.stepTextFaint)}>
            Reference to a RaisinDB function
          </p>
        </FieldGroup>
      )}

      {/* Agent Reference (for AI agent step type) */}
      {step.properties.step_type === 'ai_agent' && (
        <FieldGroup label="Agent Reference" icon={<Bot className="w-4 h-4" />}>
          <input
            type="text"
            value={getRefDisplayValue(step.properties.agent_ref)}
            onChange={(e) => updateProperty('agent_ref', createRef(e.target.value))}
            placeholder="e.g., /agents/my-agent"
            className={clsx(
              'w-full px-3 py-2 rounded-md border text-sm font-mono',
              themeClasses.stepBg,
              themeClasses.stepBorder,
              themeClasses.stepText,
              'focus:outline-none focus:ring-2 focus:ring-blue-500'
            )}
          />
        </FieldGroup>
      )}

      {/* Human Task Properties */}
      {step.properties.step_type === 'human_task' && (
        <>
          <FieldGroup label="Task Type" icon={<User className="w-4 h-4" />}>
            <select
              value={step.properties.task_type || 'approval'}
              onChange={(e) => updateProperty('task_type', e.target.value as FlowStepProperties['task_type'])}
              className={clsx(
                'w-full px-3 py-2 rounded-md border text-sm',
                themeClasses.stepBg,
                themeClasses.stepBorder,
                themeClasses.stepText,
                'focus:outline-none focus:ring-2 focus:ring-blue-500'
              )}
            >
              <option value="approval">Approval</option>
              <option value="input">Input</option>
              <option value="review">Review</option>
              <option value="action">Action</option>
            </select>
          </FieldGroup>

          <FieldGroup label="Assignee">
            <input
              type="text"
              value={step.properties.assignee || ''}
              onChange={(e) => updateProperty('assignee', e.target.value || undefined)}
              placeholder="User or group ID"
              className={clsx(
                'w-full px-3 py-2 rounded-md border text-sm',
                themeClasses.stepBg,
                themeClasses.stepBorder,
                themeClasses.stepText,
                'focus:outline-none focus:ring-2 focus:ring-blue-500'
              )}
            />
          </FieldGroup>

          <FieldGroup label="Task Description">
            <textarea
              value={step.properties.task_description || ''}
              onChange={(e) => updateProperty('task_description', e.target.value || undefined)}
              placeholder="Instructions for the assignee"
              rows={3}
              className={clsx(
                'w-full px-3 py-2 rounded-md border text-sm resize-none',
                themeClasses.stepBg,
                themeClasses.stepBorder,
                themeClasses.stepText,
                'focus:outline-none focus:ring-2 focus:ring-blue-500'
              )}
            />
          </FieldGroup>
        </>
      )}

      {/* Condition */}
      <FieldGroup label="Condition (optional)">
        <input
          type="text"
          value={step.properties.condition || ''}
          onChange={(e) => updateProperty('condition', e.target.value || undefined)}
          placeholder="e.g., $.input.value > 100"
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm font-mono',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        />
        <p className={clsx('text-xs mt-1', themeClasses.stepTextFaint)}>
          JSONPath expression that must evaluate to true
        </p>
      </FieldGroup>

      {/* Payload Key */}
      <FieldGroup label="Output Key (optional)">
        <input
          type="text"
          value={step.properties.payload_key || ''}
          onChange={(e) => updateProperty('payload_key', e.target.value || undefined)}
          placeholder="e.g., result"
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        />
        <p className={clsx('text-xs mt-1', themeClasses.stepTextFaint)}>
          Key to store step output in flow context
        </p>
      </FieldGroup>

      {/* Retry Strategy */}
      <FieldGroup label="Retry Strategy" icon={<RotateCcw className="w-4 h-4" />}>
        <select
          value={step.properties.retry_strategy || 'none'}
          onChange={(e) => updateProperty('retry_strategy', e.target.value as RetryStrategy)}
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        >
          <option value="none">No Retry</option>
          <option value="linear">Linear Backoff</option>
          <option value="exponential">Exponential Backoff</option>
          <option value="fixed">Fixed Interval</option>
        </select>
      </FieldGroup>

      {/* Timeout */}
      <FieldGroup label="Timeout (ms)" icon={<Clock className="w-4 h-4" />}>
        <input
          type="number"
          value={step.properties.timeout_ms || ''}
          onChange={(e) => updateProperty('timeout_ms', e.target.value ? parseInt(e.target.value, 10) : undefined)}
          placeholder="30000"
          min={0}
          className={clsx(
            'w-full px-3 py-2 rounded-md border text-sm',
            themeClasses.stepBg,
            themeClasses.stepBorder,
            themeClasses.stepText,
            'focus:outline-none focus:ring-2 focus:ring-blue-500'
          )}
        />
      </FieldGroup>

      {/* Disabled Toggle */}
      <div className="flex items-center justify-between">
        <span className={clsx('text-sm font-medium', themeClasses.stepText)}>
          Disabled
        </span>
        <button
          type="button"
          role="switch"
          aria-checked={step.properties.disabled || false}
          onClick={() => updateProperty('disabled', !step.properties.disabled)}
          className={clsx(
            'relative inline-flex h-6 w-11 items-center rounded-full transition-colors',
            step.properties.disabled
              ? 'bg-gray-400'
              : 'bg-blue-500'
          )}
        >
          <span
            className={clsx(
              'inline-block h-4 w-4 transform rounded-full bg-white transition-transform',
              step.properties.disabled ? 'translate-x-1' : 'translate-x-6'
            )}
          />
        </button>
      </div>
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

export default StepPropertiesEditor;
