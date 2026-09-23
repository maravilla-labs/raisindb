/**
 * read-user-context — reads the stored memory of the current user as
 * markdown. Call it before `remember` so new facts are merged, not replaced.
 *
 * Whose memory: the calling run's agent and the user it acts for, proven from
 * the run record (agent-shared/memory.js) — never from the arguments.
 */
import { memoryTool, loadUserMemory } from '../agent-shared/memory.js';

const FN = '/lib/raisin/ai/read-user-context';

async function handler(input) {
  return memoryTool(input, FN, async (owner) => ({
    payload: { content: await loadUserMemory(owner.agentName, owner.userId) },
  }));
}

export { handler };
