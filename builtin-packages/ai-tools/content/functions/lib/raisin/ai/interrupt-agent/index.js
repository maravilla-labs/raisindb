/** interrupt-agent — a delegation tool over the calling run's child runs (agent-shared/delegation-tool.js). */
import { delegationTool, interrupt } from '../agent-shared/delegation-tool.js';

export async function handler(input) {
  return delegationTool(input, interrupt);
}
