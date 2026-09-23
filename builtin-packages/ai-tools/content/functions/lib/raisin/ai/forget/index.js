/**
 * forget — removes one entry (a `- key: …` line) from the user's memory.
 * Nothing to forget is not an error: the answer says so and the call is done.
 *
 * Whose memory: the calling run's agent and the user it acts for, proven from
 * the run record (agent-shared/memory.js) — never from the arguments.
 */
import { memoryTool, loadUserMemory, saveUserMemory } from '../agent-shared/memory.js';

const FN = '/lib/raisin/ai/forget';

async function handler(input) {
  return memoryTool(input, FN, async (owner, args) => {
    const key = typeof args.key === 'string' ? args.key.trim() : '';
    if (!key) throw Object.assign(new Error('Key is required'), { error_class: 'invalid_input' });
    const current = await loadUserMemory(owner.agentName, owner.userId);
    if (!current) {
      return { payload: { success: true, found: false, key, message: 'No stored memory yet — nothing to forget.' } };
    }
    const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const pattern = new RegExp(`^-\\s+${escaped}\\s*:`);
    const lines = current.split('\n');
    const kept = lines.filter((line) => !pattern.test(line));
    if (kept.length === lines.length) {
      return { payload: { success: true, found: false, key, message: `No memory entry for "${key}" — nothing to forget.` } };
    }
    const saved = await saveUserMemory(owner.agentName, owner.userId, kept.join('\n'));
    return { payload: { success: true, found: true, key }, writes: [saved] };
  });
}

export { handler };
