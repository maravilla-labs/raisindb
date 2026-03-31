/**
 * Update Step Command
 *
 * Command to update step properties.
 */

import { AbstractCommand } from './AbstractCommand';
import type { CommandContext, UpdateStepParams, FlowStep } from '../types';
import { cloneFlow, findNodeAndParent, isFlowStep } from '../utils';

export class UpdateStepCommand extends AbstractCommand {
  private params: UpdateStepParams;

  constructor(context: CommandContext, params: UpdateStepParams) {
    super(context, 'UPDATE_STEP', 'Update step');
    this.params = params;
  }

  execute(): void {
    this.saveState();

    this.context.setState((currentState) => {
      const newState = cloneFlow(currentState);
      const { nodeId, updates } = this.params;

      const result = findNodeAndParent(newState, nodeId);
      if (!result || !isFlowStep(result.node)) {
        return currentState;
      }

      const step = result.node as FlowStep;

      // Apply property updates - handle all FlowStepProperties fields
      // Core step properties
      if (updates.action !== undefined) {
        step.properties.action = updates.action;
      }
      if ('function_ref' in updates) {
        step.properties.function_ref = updates.function_ref;
      }
      if ('agent_ref' in updates) {
        step.properties.agent_ref = updates.agent_ref;
      }
      if (updates.lua_script !== undefined) {
        step.properties.lua_script = updates.lua_script;
      }
      if (updates.condition !== undefined) {
        step.properties.condition = updates.condition;
      }
      if (updates.payload_key !== undefined) {
        step.properties.payload_key = updates.payload_key;
      }
      if (updates.disabled !== undefined) {
        step.properties.disabled = updates.disabled;
      }
      if (updates.on_error !== undefined) {
        step.on_error = updates.on_error;
      }

      // Human task properties
      if (updates.task_type !== undefined) {
        step.properties.task_type = updates.task_type;
      }
      if (updates.assignee !== undefined) {
        step.properties.assignee = updates.assignee;
      }
      if (updates.task_description !== undefined) {
        step.properties.task_description = updates.task_description;
      }
      if ('options' in updates) {
        step.properties.options = updates.options;
      }
      if (updates.priority !== undefined) {
        step.properties.priority = updates.priority;
      }
      if (updates.due_in_seconds !== undefined) {
        step.properties.due_in_seconds = updates.due_in_seconds;
      }

      // Retry configuration
      if (updates.retry_strategy !== undefined) {
        step.properties.retry_strategy = updates.retry_strategy;
      }
      if ('retry' in updates) {
        step.properties.retry = updates.retry;
      }
      if (updates.timeout_ms !== undefined) {
        step.properties.timeout_ms = updates.timeout_ms;
      }

      // Error handling and execution identity
      if (updates.error_edge !== undefined) {
        step.properties.error_edge = updates.error_edge;
      }
      if ('compensation_ref' in updates) {
        step.properties.compensation_ref = updates.compensation_ref;
      }
      if (updates.continue_on_fail !== undefined) {
        step.properties.continue_on_fail = updates.continue_on_fail;
      }
      if (updates.isolated_branch !== undefined) {
        step.properties.isolated_branch = updates.isolated_branch;
      }
      if (updates.execution_identity !== undefined) {
        step.properties.execution_identity = updates.execution_identity;
      }

      // Chat step configuration
      if ('chat_config' in updates) {
        step.properties.chat_config = updates.chat_config;
      }

      return newState;
    });
  }

  undo(): void {
    this.restoreState();
  }
}
