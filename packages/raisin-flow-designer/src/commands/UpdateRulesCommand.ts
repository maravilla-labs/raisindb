/**
 * Update Container Rules Command
 *
 * Command to update container rules and type.
 */

import { AbstractCommand } from './AbstractCommand';
import type { CommandContext, UpdateContainerParams, FlowContainer } from '../types';
import { cloneFlow, findNodeAndParent, isFlowContainer } from '../utils';

export class UpdateRulesCommand extends AbstractCommand {
  private params: UpdateContainerParams;

  constructor(context: CommandContext, params: UpdateContainerParams) {
    super(context, 'UPDATE_CONTAINER', 'Update container');
    this.params = params;
  }

  execute(): void {
    this.saveState();

    this.context.setState((currentState) => {
      const newState = cloneFlow(currentState);
      const { containerId, container_type, rules, ai_config, timeout_ms } = this.params;

      const result = findNodeAndParent(newState, containerId);
      if (!result || !isFlowContainer(result.node)) {
        return currentState;
      }

      const container = result.node as FlowContainer;

      // Apply updates
      if (container_type !== undefined) {
        container.container_type = container_type;
      }
      if (rules !== undefined) {
        container.rules = rules;
      }
      if (ai_config !== undefined) {
        container.ai_config = ai_config;
      }
      if (timeout_ms !== undefined) {
        container.timeout_ms = timeout_ms;
      }

      return newState;
    });
  }

  undo(): void {
    this.restoreState();
  }
}
