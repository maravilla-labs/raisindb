/**
 * remember — saves the user's memory as one complete markdown document,
 * replacing the previous one (read it first with read-user-context, merge,
 * then pass the whole document).
 *
 * Whose memory: the calling run's agent and the user it acts for, proven from
 * the run record (agent-shared/memory.js) — never from the arguments.
 */
import { memoryTool, saveUserMemory } from '../agent-shared/memory.js';

const FN = '/lib/raisin/ai/remember';

async function handler(input) {
  return memoryTool(input, FN, async (owner, args) => {
    const content = typeof args.content === 'string' ? args.content.trim() : '';
    if (!content) throw Object.assign(new Error('Non-empty markdown content is required'), { error_class: 'invalid_input' });
    const saved = await saveUserMemory(owner.agentName, owner.userId, content);
    return { payload: { success: true, chars: content.length }, writes: [saved] };
  });
}

export { handler };
