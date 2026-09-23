/** wait-for-agents — a delegation tool over the calling run's child runs (agent-shared/delegation-tool.js). */
import { delegationTool, wait } from '../agent-shared/delegation-tool.js';

export async function handler(input) {
  return delegationTool(input, wait);
}
