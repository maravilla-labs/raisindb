/**
 * spawn-agent — start a durable CHILD RUN of an installed agent for the
 * calling run (see agent-shared/delegation-spawn.js). Answers at once with the
 * child's run id; the result comes back through wait-for-agents.
 */
import { delegationTool } from '../agent-shared/delegation-tool.js';
import { spawnChild } from '../agent-shared/delegation-spawn.js';
import { DELEGATION_FUNCTIONS } from '../agent-shared/delegation-spec.js';
import { locator } from '../agent-shared/tool-envelope.js';

export async function handler(input) {
  return delegationTool(input, async (args) => {
    const r = await spawnChild(args, { functionPath: DELEGATION_FUNCTIONS.spawn });
    const loc = locator('ai', r.chat_path);
    return {
      payload: {
        child: r.child,
        replayed: r.replayed,
        live_children: r.live_children,
        message: `Started child run ${r.child.key} (${r.child.agent_ref}). It works in the background; collect its result with wait_for_agents.`,
      },
      writes: r.replayed ? [] : [{ locator: loc, action: 'created' }],
      refs: [{ kind: 'child_run', logical_key: r.child.key, locator: loc, role: 'primary' }],
      next: [{ action: 'wait_for_agents', args: { agents: [r.child.key] }, reason: 'the child runs asynchronously' }],
    };
  });
}
