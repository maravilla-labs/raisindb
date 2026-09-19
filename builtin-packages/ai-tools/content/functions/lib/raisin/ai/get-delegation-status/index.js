async function handler(input) {
  const { task_id, __raisin_context } = input;
  const workspace = __raisin_context?.workspace || 'ai';
  const chatPath = __raisin_context?.chat_path;
  if (!task_id) throw new Error('task_id is required');
  if (!chatPath) throw new Error('Missing chat_path in execution context');

  const rows = await raisin.sql.query(
    `SELECT id, path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AITask' AND id = $2
     LIMIT 1`,
    [chatPath, task_id],
  );
  if (!rows.length) return { found: false, task_id, status: 'missing' };
  const props = rows[0].properties || {};
  return {
    found: true,
    task_id,
    title: props.title || '',
    status: props.delegation_status || (props.delegation_id ? 'unknown' : 'not_delegated'),
    task_status: props.status || 'pending',
    delegation_id: props.delegation_id || null,
    flow_instance_id: props.delegation_flow_instance_id || null,
    agent_ref: props.delegated_agent_ref || null,
    branch: props.delegation_branch || null,
    base_branch: props.delegation_base_branch || null,
    result: props.delegation_result || null,
    error: props.delegation_error || null,
  };
}
