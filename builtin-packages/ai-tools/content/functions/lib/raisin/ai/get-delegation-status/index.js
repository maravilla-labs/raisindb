/**
 * get-delegation-status — the plan-task spelling of `inspect-agent`: the
 * child run delegated for a task (read from core), with its status, outcome,
 * summary, artifacts and acceptance.
 */
import { delegationTool, inspect } from '../agent-shared/delegation-tool.js';
import { DELEGATION_FUNCTIONS } from '../agent-shared/delegation-spec.js';

export async function handler(input) {
  return delegationTool(input, async (args) => {
    if (!args.task_id) throw Object.assign(new Error('task_id is required'), { error_class: 'invalid_input' });
    const r = await inspect({ ...args, agent: args.task_id }, DELEGATION_FUNCTIONS.legacyStatus);
    const c = r.payload.child;
    return { payload: { found: true, task_id: args.task_id, status: c.status, outcome: c.outcome, result: c.summary, child: c } };
  });
}
