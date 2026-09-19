/** Start a durable child-agent run for one task in the current plan. */
async function handler(input) {
  const { task_id, agent_ref, objective, context, max_tool_iterations, __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const chatPath = __raisin_context?.chat_path;

  if (!chatPath) throw new Error('Missing chat_path in execution context');
  if (!task_id || !agent_ref || !objective) {
    throw new Error('task_id, agent_ref, and objective are required');
  }
  if (!/^\/agents\/[a-z0-9][a-z0-9-]*$/.test(agent_ref)) {
    throw new Error('agent_ref must be an installed /agents/{slug} path');
  }

  const agent = await raisin.nodes.get('functions', agent_ref);
  if (!agent || agent.node_type !== 'raisin:AIAgent') {
    throw new Error(`Delegation target is not an installed agent: ${agent_ref}`);
  }

  const rows = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AITask' AND id = $2
     LIMIT 1`,
    [chatPath, task_id],
  );
  if (!rows.length) throw new Error(`Task not found in current conversation: ${task_id}`);

  const task = rows[0];
  const delegationId = await raisin.crypto.uuid();
  const startedAt = new Date().toISOString();
  const updated = {
    ...(task.properties || {}),
    status: 'in_progress',
    delegation_id: delegationId,
    delegation_status: 'queued',
    delegated_agent_ref: {
      'raisin:ref': agent.id || '',
      'raisin:workspace': 'functions',
      'raisin:path': agent_ref,
    },
    delegated_at: startedAt,
    delegation_objective: objective,
  };
  await raisin.nodes.update(workspace, task.path, { properties: updated });

  let run;
  try {
    run = await raisin.flows.run('/flows/delegated-agent-task', {
      delegation_id: delegationId,
      task_id,
      task_path: task.path,
      chat_path: chatPath,
      workspace,
      agent_ref,
      objective,
      context: context || {},
      max_tool_iterations: Math.min(Math.max(Number(max_tool_iterations) || 6, 1), 12),
    });
  } catch (error) {
    await raisin.nodes.update(workspace, task.path, {
      properties: {
        ...updated,
        delegation_status: 'failed',
        delegation_error: String(error?.message || error),
      },
    });
    throw error;
  }

  const instanceId = run?.instance_id || run?.instanceId || null;
  await raisin.nodes.update(workspace, task.path, {
    properties: {
      ...updated,
      delegation_status: 'running',
      delegation_flow_instance_id: instanceId,
    },
  });

  return {
    success: true,
    delegation_id: delegationId,
    flow_instance_id: instanceId,
    task_id,
    agent_ref,
    status: 'running',
    message: `Delegated "${updated.title || task_id}" to ${agent.properties?.title || agent_ref}.`,
  };
}
