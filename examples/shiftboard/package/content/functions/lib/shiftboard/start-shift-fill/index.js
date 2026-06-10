/**
 * start-shift-fill - agent tool that starts the durable fill-shift workflow.
 *
 * Instead of the agent chatting with each staff member itself, this kicks
 * off the /flows/fill-shift raisin:Flow: the engine asks each available AND
 * reachable candidate via an inbox approval task (accept/decline buttons,
 * with a deadline), assigns the first accepter, and reports the outcome to
 * the manager - durable, auditable, and independent of the chat session.
 *
 * Uses the function-runtime flow capability raisin.flows.run (fire and
 * forget) - the same engine path as POST /api/flows/{repo}/run.
 *
 * Input:  { shift_path: string }  e.g. "/shifts/sun-evening"
 * Output: { instance_id, status, message }
 */
async function handler(input) {
  const { shift_path } = input || {};
  if (!shift_path || shift_path.indexOf('/shifts/') !== 0) {
    throw new Error('shift_path is required and must start with /shifts/, got: ' + shift_path);
  }

  const run = await raisin.flows.run('/flows/fill-shift', { shift_path });

  console.log('[start-shift-fill] started /flows/fill-shift for', shift_path,
    'instance', run.instance_id);

  return {
    instance_id: run.instance_id,
    status: run.status || 'queued',
    message:
      'Fill-shift workflow started for ' + shift_path +
      '. Available, reachable staff will receive accept/decline tasks in ' +
      'their inbox one by one; the manager is notified of the outcome.',
  };
}
