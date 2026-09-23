/**
 * delegate-task — the plan-task spelling of `spawn-agent`: a durable CHILD RUN
 * of core (see agent-shared/delegation-spawn.js), keyed by the task, so
 * delegating the same task twice finds the first child instead of starting
 * another. The parent inspects, messages, waits for and interrupts it like
 * any child; its hand-back arrives through the run mailbox.
 */
import { delegationTool } from '../agent-shared/delegation-tool.js';
import { spawnChild } from '../agent-shared/delegation-spawn.js';
import { DELEGATION_FUNCTIONS, taskKey } from '../agent-shared/delegation-spec.js';
import { locator } from '../agent-shared/tool-envelope.js';

export async function handler(input) {
  return delegationTool(input, async (args) => {
    const { task_id, agent_ref, objective, context, max_tool_iterations } = args;
    if (!task_id || !agent_ref || !objective) throw Object.assign(new Error('task_id, agent_ref, and objective are required'), { error_class: 'invalid_input' });
    const iterations = Math.min(Math.max(Number(max_tool_iterations) || 6, 1), 12);
    const r = await spawnChild({
      __raisin_context: args.__raisin_context,
      agent_ref,
      key: taskKey(task_id),
      task_id,
      objective,
      context_mode: context ? 'snapshot' : 'none',
      context: context || null,
      budget: { max_model_calls: iterations + 2, max_operations: iterations * 4 + 8 },
      independent: args.independent === true,
    }, { functionPath: DELEGATION_FUNCTIONS.legacyDelegate, taskBinding: task_id });
    const loc = locator('ai', r.chat_path);
    return {
      payload: {
        success: true,
        delegation_id: r.child.run_id,
        child_run_id: r.child.run_id,
        child: r.child,
        task_id,
        agent_ref,
        status: r.child.status || 'running',
        replayed: r.replayed,
        message: `Delegated task ${task_id} to ${agent_ref} as child run ${r.child.key}. Collect its result with wait_for_agents or get_delegation_status.`,
      },
      writes: r.replayed ? [] : [{ locator: loc, action: 'created' }],
      refs: [{ kind: 'child_run', logical_key: r.child.key, locator: loc, role: 'primary' }],
      next: [{ action: 'wait_for_agents', args: { agents: [r.child.key] }, reason: 'the child runs asynchronously' }],
    };
  });
}
