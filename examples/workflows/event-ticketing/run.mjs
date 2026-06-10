#!/usr/bin/env node
/**
 * Event ticketing workflow - end-to-end demo with the @raisindb/client SDK.
 *
 * What it shows:
 *   1. JS functions deployed as raisin:Function nodes (code in a child
 *      index.js raisin:Asset node) under /lib/ticketing/
 *   2. A designer-format flow /flows/ticket-order:
 *        reserve  - function step with saga compensation (cancel-reservation)
 *        gate     - OR container: REL rules route big/VIP orders to a
 *                   human approval task; small orders fall through
 *        issue    - function step consuming ${steps.reserve.reservation_id}
 *   3. Two live scenarios:
 *        (a) standard order  -> completes WITHOUT approval
 *        (b) VIP order       -> pauses on approval -> inbox -> approve -> done
 *
 * Prereqs: a running raisin-server (dev mode) on RAISIN_URL.
 *   RUST_LOG=info ./target/release/raisin-server --config ... --dev-mode
 *
 * Run:
 *   npm install && npm start
 */

import { readFile } from 'node:fs/promises';
import { RaisinHttpClient, FlowClient, InboxApi } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'ticketing-demo';
const USERNAME = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const FLOW_PATH = '/flows/ticket-order';
const FUNCTIONS = ['reserve-seats', 'issue-tickets', 'cancel-reservation'];

// The workflow definition in DESIGNER format - the same format the admin
// console's visual flow designer reads and writes. Execution order is the
// array order; the engine injects start/end and lowers containers.
const workflowData = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    // 1. Reserve seats. Registers saga compensation once it succeeds:
    //    if a LATER step fails unrecoverably, cancel-reservation runs
    //    with the mapped input (output.* = this step's fresh output).
    {
      id: 'reserve',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Reserve {{ input.quantity }}x {{ input.tier }} for {{ input.event_id }}',
        function_ref: '/lib/ticketing/reserve-seats',
        arguments: {
          event_id: '{{ input.event_id }}',
          quantity: '${input.quantity}', // whole-string expression keeps the number type
          tier: '{{ input.tier }}',
        },
        compensation_ref: '/lib/ticketing/cancel-reservation',
        compensation_input_mapping: {
          reservation_id: '${output.reservation_id}',
        },
        timeout_ms: 30000,
      },
    },

    // 2. Approval gate. Rules are evaluated in order against the flow
    //    context; the first match routes to its child. If NO rule matches
    //    (small standard orders) the whole container is skipped.
    {
      id: 'approval-gate',
      node_type: 'raisin:FlowContainer',
      container_type: 'or',
      rules: [
        { condition: 'steps.reserve.total_price > 500', next_step: 'approve' },
        { condition: 'input.tier == "vip"', next_step: 'approve' },
      ],
      children: [
        {
          id: 'approve',
          node_type: 'raisin:FlowStep',
          properties: {
            action:
              'Approve {{ input.quantity }}x {{ input.tier }} ticket order ({{ steps.reserve.total_price }} CHF)',
            step_type: 'human_task',
            task_type: 'approval',
            assignee: '/users/admin',
            task_description:
              'Order for event {{ input.event_id }} needs approval. ' +
              'Reservation {{ steps.reserve.reservation_id }}, total {{ steps.reserve.total_price }} CHF.',
            priority: 4,
            options: [
              { value: 'approve', label: 'Approve', style: 'success' },
              { value: 'reject', label: 'Reject', style: 'danger' },
            ],
          },
        },
      ],
    },

    // 3. Issue the tickets, referencing the reservation made in step 1.
    //    PITFALL (two independent retry layers):
    //      - the queued function-execution JOB retries failures 3 times on
    //        a fixed ~10s/30s backoff regardless of the step's retry config,
    //      - then the FLOW retries the whole step (default 3 more times)
    //        unless retry_strategy 'none' disables it.
    //    timeout_ms is the flow's wait deadline - it must outlive the job
    //    retry schedule (~40s), otherwise the wait expires first, the flow
    //    fails via timeout, and saga compensation never runs.
    {
      id: 'issue',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Issue tickets for {{ steps.reserve.reservation_id }}',
        function_ref: '/lib/ticketing/issue-tickets',
        arguments: {
          reservation_id: '${steps.reserve.reservation_id}',
          quantity: '${input.quantity}',
          fail: '${input.simulate_issue_failure}', // null/false in normal runs
        },
        retry_strategy: 'none',
        timeout_ms: 120000,
      },
    },
  ],
};

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function api(client, method, path, body) {
  const token = client.getAuthManager().getAccessToken();
  return fetch(`${BASE_URL}${path}`, {
    method,
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: body ? JSON.stringify(body) : undefined,
  });
}

// ---------------------------------------------------------------------------
// Setup (idempotent, with retries - node types initialize asynchronously
// right after repository creation, so first writes can fail transiently)
// ---------------------------------------------------------------------------

/** Create a node; on "already exists" refresh its properties via PUT. */
async function ensureNode(client, parentPath, name, nodeType, properties) {
  const base = `/api/repository/${REPO}/main/head/functions`;
  let lastError = '';
  for (let i = 0; i < 20; i++) {
    const created = await api(client, 'POST', `${base}${parentPath}`, {
      node: { name, node_type: nodeType, properties },
    });
    if (created.ok) return 'created';
    lastError = await created.text();
    if (/exists|conflict/i.test(lastError)) {
      const childPath =
        parentPath === '/' ? `/${name}` : `${parentPath.replace(/\/$/, '')}/${name}`;
      const updated = await api(client, 'PUT', `${base}${childPath}`, { properties });
      if (updated.ok) return 'updated';
      // Folders may reject property updates - reuse as-is.
      return 'reused';
    }
    await sleep(1000);
  }
  throw new Error(`Failed to create ${parentPath}/${name}: ${lastError}`);
}

/**
 * Deploy a JS function as nodes:
 *   /lib/ticketing/<name>           raisin:Function (metadata, entry_file)
 *   /lib/ticketing/<name>/index.js  raisin:Asset with the source in the
 *                                   inline `code` property (the same place
 *                                   the Functions IDE and packages put it)
 */
async function deployFunction(client, name, title) {
  const code = await readFile(new URL(`./functions/${name}.js`, import.meta.url), 'utf8');
  await ensureNode(client, '/lib/ticketing', name, 'raisin:Function', {
    name,
    title,
    description: `${title} (event-ticketing example)`,
    enabled: true,
    language: 'javascript',
    execution_mode: 'async',
    entry_file: 'index.js:handler',
    version: 1,
  });
  await ensureNode(client, `/lib/ticketing/${name}`, 'index.js', 'raisin:Asset', {
    title: 'index.js',
    file: '', // raisin:Asset requires 'file'; the source lives in the inline 'code' property
    code,
  });
  console.log(`✅ Function deployed: /lib/ticketing/${name}`);
}

/** Smoke-test a function via POST /api/functions/{repo}/{name}/invoke. */
async function invokeFunction(client, name, input) {
  const res = await api(client, 'POST', `/api/functions/${REPO}/${name}/invoke`, {
    input,
    wait_for_completion: true,
    wait_timeout_ms: 30000,
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok || body.error) {
    const err = new Error(
      `invoke ${name} failed: HTTP ${res.status} ${JSON.stringify(body)}`,
    );
    err.status = res.status;
    throw err;
  }
  // The job result may wrap the function's return value as { success, result }.
  const raw = body.result ?? {};
  return raw && typeof raw === 'object' && 'result' in raw && 'success' in raw
    ? raw.result
    : raw;
}

/**
 * Fallback smoke test via POST /api/files/{repo}/run (direct file execution,
 * SSE response). Needed because of a current engine bug: the /api/functions
 * lookup (`find_function_node` in raisin-transport-http) builds its node
 * service WITHOUT the caller's auth context, so RLS denies every function
 * node and list/get/invoke return empty/404 even for admins. Flow execution
 * is unaffected (the flow runtime loads functions via storage directly).
 */
async function runFileDirect(client, functionName, input) {
  const fileRes = await api(
    client,
    'GET',
    `/api/repository/${REPO}/main/head/functions/lib/ticketing/${functionName}/index.js`,
  );
  if (!fileRes.ok) throw new Error(`could not load index.js node for ${functionName}`);
  const { id: nodeId } = await fileRes.json();

  const res = await api(client, 'POST', `/api/files/${REPO}/run`, {
    node_id: nodeId,
    handler: 'handler',
    input,
  });
  if (!res.ok) throw new Error(`files/run failed: HTTP ${res.status} ${await res.text()}`);

  // Parse the SSE stream for the `result` event
  const text = await res.text();
  for (const line of text.split('\n')) {
    if (!line.startsWith('data:')) continue;
    try {
      const event = JSON.parse(line.slice(5));
      if (event.type === 'result') {
        if (!event.success) throw new Error(`function failed: ${event.error}`);
        return event.result;
      }
    } catch (e) {
      if (e instanceof SyntaxError) continue;
      throw e;
    }
  }
  throw new Error('files/run produced no result event');
}

/** Prove a function executes, preferring the invoke API with a fallback. */
async function smokeTestFunction(client, name, input) {
  try {
    const out = await invokeFunction(client, name, input);
    return { via: `/api/functions/${REPO}/${name}/invoke`, out };
  } catch (err) {
    if (err.status !== 404) throw err;
    console.log(
      `⚠️  /api/functions invoke returned 404 (known engine bug: function ` +
        `lookup ignores auth context, RLS denies) - falling back to /api/files/run`,
    );
    const out = await runFileDirect(client, name, input);
    return { via: `/api/files/${REPO}/run`, out };
  }
}

async function ensureSetup(client) {
  // Repository (idempotent - ignore "already exists")
  await api(client, 'POST', '/api/repositories', { repo_id: REPO });

  // Folders in the functions workspace
  await ensureNode(client, '/', 'lib', 'raisin:Folder', {});
  await ensureNode(client, '/lib', 'ticketing', 'raisin:Folder', {});
  await ensureNode(client, '/', 'flows', 'raisin:Folder', {});

  // Functions
  await deployFunction(client, 'reserve-seats', 'Reserve Seats');
  await deployFunction(client, 'issue-tickets', 'Issue Tickets');
  await deployFunction(client, 'cancel-reservation', 'Cancel Reservation');

  // Prove a function actually EXECUTES before relying on the flow
  const { via, out: smoke } = await smokeTestFunction(client, 'reserve-seats', {
    event_id: 'SMOKE-TEST',
    quantity: 1,
    tier: 'standard',
  });
  assert(
    smoke && smoke.total_price === 50 && typeof smoke.reservation_id === 'string',
    `reserve-seats smoke test returned ${JSON.stringify(smoke)}`,
  );
  console.log(
    `✅ Function smoke test passed via ${via} (reservation ${smoke.reservation_id}, 50 CHF)`,
  );

  // The flow node
  const state = await ensureNode(client, '/flows', 'ticket-order', 'raisin:Flow', {
    name: 'ticket-order',
    title: 'Ticket Order',
    description: 'Reserve seats, route big/VIP orders to approval, issue tickets.',
    enabled: true,
    workflow_data: workflowData,
  });
  console.log(
    state === 'created'
      ? `✅ Flow deployed at ${FLOW_PATH}`
      : `ℹ️  Flow node already existed - definition refreshed (${state})`,
  );
}

// ---------------------------------------------------------------------------
// Flow helpers
// ---------------------------------------------------------------------------

async function waitForStatus(flows, instanceId, wanted, attempts = 120) {
  let last = 'unknown';
  for (let i = 0; i < attempts; i++) {
    try {
      const status = await flows.getInstanceStatus(instanceId);
      last = status.status;
      if (wanted.includes(status.status)) return status;
    } catch {
      // instance node may not be visible yet - keep polling
    }
    await sleep(500);
  }
  throw new Error(
    `Timed out waiting for ${wanted.join('/')} (last status: ${last}) on ${instanceId}`,
  );
}

async function findTask(inbox, instanceId, attempts = 30) {
  for (let i = 0; i < attempts; i++) {
    const { tasks } = await inbox.listTasks({
      status: 'pending',
      assignee: '/users/admin',
    });
    const task = tasks.find((t) => t.flow_instance_id === instanceId);
    if (task) return task;
    await sleep(500);
  }
  throw new Error('Approval task did not appear in the inbox');
}

function stepOutputs(status) {
  return status.variables?.step_outputs ?? {};
}

// ---------------------------------------------------------------------------
// Scenario A: standard order - no approval needed
// ---------------------------------------------------------------------------

async function runStandardOrder(flows) {
  console.log('\n━━━ Scenario A: standard order (2x standard = 100 CHF) ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    event_id: 'EVT-2026-ROCKNIGHT',
    quantity: 2,
    tier: 'standard',
  });
  console.log('✅ Flow started:', instance_id);

  // NOTE: the instance status reads 'waiting' transiently while each queued
  // function executes (wait_type=function_call), so we wait for a TERMINAL
  // status here and verify "no approval happened" via the variables below.
  const status = await waitForStatus(flows, instance_id, ['completed', 'failed']);
  assert(
    status.status === 'completed',
    `standard order must complete without approval, got: ${status.status} ${status.error ?? ''}`,
  );
  console.log('✅ Flow completed without pausing for approval');

  const outputs = stepOutputs(status);
  const reserve = outputs.reserve ?? {};
  const issue = outputs.issue ?? {};

  assert(reserve.total_price === 100, `expected total_price 100, got ${reserve.total_price}`);
  assert(reserve.tier === 'standard', `expected tier standard, got ${reserve.tier}`);
  console.log(`  💰 reserve: ${reserve.reservation_id} -> ${reserve.total_price} CHF`);

  assert(
    Array.isArray(issue.ticket_ids) && issue.ticket_ids.length === 2,
    `issue-tickets must have run and issued 2 tickets, got ${JSON.stringify(issue)}`,
  );
  assert(
    issue.reservation_id === reserve.reservation_id,
    'tickets must reference the reservation from the reserve step',
  );
  console.log(`  🎫 issued: ${issue.ticket_ids.join(', ')}`);

  assert(
    outputs.approve === undefined && status.variables?.__human_response === undefined,
    'approval step must have been skipped for a standard order',
  );
  console.log('✅ Approval gate was skipped (no rule matched)');
}

// ---------------------------------------------------------------------------
// Scenario B: VIP order - pauses for approval, completed via the inbox
// ---------------------------------------------------------------------------

async function runVipOrder(flows, inbox) {
  console.log('\n━━━ Scenario B: VIP order (4x vip = 600 CHF) ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    event_id: 'EVT-2026-GALA',
    quantity: 4,
    tier: 'vip',
  });
  console.log('✅ Flow started:', instance_id);

  // 'waiting' alone is ambiguous (function executions also wait briefly),
  // so the definitive pause signal is the approval task showing up in the
  // assignee's inbox. Once it exists, the instance must be 'waiting'.
  const task = await findTask(inbox, instance_id);
  console.log(`  📥 inbox task: "${task.title}" [${task.task_type}, P${task.priority}]`);
  assert(task.task_type === 'approval', `expected an approval task, got ${task.task_type}`);

  const waiting = await flows.getInstanceStatus(instance_id);
  assert(
    waiting.status === 'waiting',
    `VIP order must pause for approval, got: ${waiting.status} ${waiting.error ?? ''}`,
  );
  console.log('✅ Flow is waiting for approval');

  const approval = {
    action: 'approve',
    comment: 'VIP order approved from the event-ticketing example',
  };
  const result = await inbox.completeTask(task.id, approval);
  console.log('✅ Task approved, flow resuming (job', result.flow?.job_id + ')');

  // RECOVERY for an engine race: a job worker can grab the queued resume
  // job before its JobDataStore context is written ("Missing job context"
  // in the server log) - the job is then dropped and the flow stays
  // 'waiting' forever. If that happens, re-issue the resume via the public
  // resume endpoint (it feeds the same resume path as inbox completion).
  let final;
  try {
    final = await waitForStatus(flows, instance_id, ['completed', 'failed'], 20);
  } catch {
    console.log(
      '⚠️  flow still waiting after approval (known engine race: resume job ' +
        'dropped on "Missing job context") - re-issuing resume',
    );
    await flows.resume(instance_id, { ...approval, completed_by: 'admin' });
    final = await waitForStatus(flows, instance_id, ['completed', 'failed']);
  }
  assert(
    final.status === 'completed',
    `VIP order must complete after approval, got: ${final.status} ${final.error ?? ''}`,
  );

  const outputs = stepOutputs(final);
  const reserve = outputs.reserve ?? {};
  const issue = outputs.issue ?? {};
  const decision = final.variables?.__human_response ?? {};

  assert(reserve.total_price === 600, `expected total_price 600, got ${reserve.total_price}`);
  console.log(`  💰 reserve: ${reserve.reservation_id} -> ${reserve.total_price} CHF`);

  assert(
    decision.action === 'approve',
    `expected __human_response.action approve, got ${JSON.stringify(decision)}`,
  );
  console.log(`  👤 decision: ${decision.action} by ${decision.completed_by}`);

  assert(
    Array.isArray(issue.ticket_ids) && issue.ticket_ids.length === 4,
    `issue-tickets must have issued 4 tickets, got ${JSON.stringify(issue)}`,
  );
  assert(
    issue.reservation_id === reserve.reservation_id,
    'tickets must reference the reservation from the reserve step',
  );
  console.log(`  🎫 issued: ${issue.ticket_ids.join(', ')}`);
  console.log('✅ Flow completed after approval');
}

// ---------------------------------------------------------------------------
// Scenario C: downstream failure - saga compensation cancels the reservation
// ---------------------------------------------------------------------------

async function runCompensatedOrder(flows, client) {
  console.log('\n━━━ Scenario C: issue-tickets outage -> saga rollback ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    event_id: 'EVT-2026-FAILTEST',
    quantity: 1,
    tier: 'standard',
    simulate_issue_failure: true, // makes the issue step throw after reserve succeeded
  });
  console.log('✅ Flow started:', instance_id);

  // The failing function is retried by the job system (~40s total) before
  // the flow sees the final failure and rolls back - poll up to 2 minutes.
  console.log('  ⏳ waiting for the job retries to exhaust (~40s) ...');
  const final = await waitForStatus(
    flows,
    instance_id,
    ['rolled_back', 'failed', 'completed'],
    240,
  );
  assert(
    final.status === 'rolled_back',
    `expected saga rollback, got: ${final.status} ${final.error ?? ''} ` +
      `(variables: ${JSON.stringify(final.variables)})`,
  );
  console.log(`✅ Flow rolled back (error: ${final.error ?? 'simulated outage'})`);

  const outputs = stepOutputs(final);
  const reserve = outputs.reserve ?? {};
  assert(
    typeof reserve.reservation_id === 'string',
    'reserve must have succeeded before the rollback',
  );
  assert(outputs.issue === undefined, 'issue step must NOT have produced output');

  // The instance node (raisin:system workspace) records the saga stack -
  // assert the compensation actually EXECUTED with the mapped input.
  const nodeRes = await api(
    client,
    'GET',
    `/api/repository/${REPO}/main/head/raisin:system/flows/instances/${instance_id}`,
  );
  assert(nodeRes.ok, `could not read instance node: HTTP ${nodeRes.status}`);
  const instanceNode = await nodeRes.json();
  const stack = instanceNode.properties?.compensation_stack ?? [];
  const entry = stack.find((e) => e.step_id === 'reserve');
  assert(entry, `no compensation entry for 'reserve' in ${JSON.stringify(stack)}`);
  assert(
    entry.compensation_status?.status === 'executed',
    `compensation must be executed, got ${JSON.stringify(entry.compensation_status)}`,
  );
  assert(
    entry.compensation_input?.reservation_id === reserve.reservation_id,
    'compensation_input_mapping must pass ${output.reservation_id} to cancel-reservation',
  );
  console.log(
    `  ↩️  compensation executed: ${entry.compensation_fn} ` +
      `({ reservation_id: ${entry.compensation_input.reservation_id} })`,
  );
}

// ---------------------------------------------------------------------------

async function main() {
  const client = new RaisinHttpClient(BASE_URL, { tenantId: 'default' });
  await client.authenticate({ username: USERNAME, password: PASSWORD });
  console.log('✅ Authenticated as', USERNAME);

  await ensureSetup(client);

  const flows = FlowClient.fromHttpClient(client, BASE_URL, REPO);
  const inbox = new InboxApi(BASE_URL, REPO, client.getAuthManager());

  await runStandardOrder(flows);
  await runVipOrder(flows, inbox);
  await runCompensatedOrder(flows, client);

  console.log('\n🎉 All scenarios passed.');
}

main().catch((err) => {
  console.error('❌', err.message ?? err);
  process.exit(1);
});
