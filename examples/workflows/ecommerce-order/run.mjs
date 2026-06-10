#!/usr/bin/env node
/**
 * Ecommerce order fulfillment - end-to-end SAGA demo with the
 * @raisindb/client SDK.
 *
 * What it shows:
 *   1. JS functions deployed as raisin:Function nodes (code in a child
 *      index.js raisin:Asset node) under /lib/ecommerce/
 *   2. A designer-format flow /flows/fulfill-order:
 *        charge_payment - function step, saga compensation refund-payment
 *        fraud_gate     - OR container: high-value / flagged orders go to a
 *                         human fraud-review task (release | cancel)
 *        routing_gate   - OR container: "cancel" voids the charge and skips
 *                         fulfillment; everything else enters the fulfill
 *                         AND-container:
 *          allocate_stock - function step, saga compensation release-stock
 *          ship_order     - function step (carrier outage simulation)
 *   3. Four live scenarios:
 *        (a) normal order     -> no review, charged + allocated + shipped
 *        (b) high-value order -> fraud review -> release -> shipped
 *        (c) carrier outage   -> saga rollback, BOTH compensations in LIFO
 *                                order (release-stock first, then refund)
 *        (d) fraud cancel     -> charge voided, fulfillment skipped
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
const REPO = process.env.RAISIN_REPO ?? 'ecommerce-demo';
const USERNAME = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const FLOW_PATH = '/flows/fulfill-order';

// The workflow definition in DESIGNER format - the same format the admin
// console's visual flow designer reads and writes. Execution order is the
// array order; the engine injects start/end and lowers containers.
//
// PITFALL (step ids): REL conditions and ${steps...} templates cannot
// dot-reference hyphenated ids - the parser reads `steps.charge-payment`
// as `steps.charge MINUS payment`. Bracket access steps["charge-payment"]
// parses but is NOT null-safe (errors when the step has not run, which
// breaks rules that must evaluate before/without that step). Use
// snake_case step ids; dot access on them is null-safe.
const workflowData = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    // 1. Charge the payment. Registers saga compensation once it succeeds:
    //    if a LATER step fails unrecoverably, refund-payment runs with the
    //    mapped input (output.* = this step's fresh output).
    {
      id: 'charge_payment',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Charge {{ input.total }} CHF for order {{ input.order_id }}',
        function_ref: '/lib/ecommerce/charge-payment',
        arguments: {
          order_id: '{{ input.order_id }}',
          amount: '${input.total}', // whole-string expression keeps the number type
        },
        compensation_ref: '/lib/ecommerce/refund-payment',
        compensation_input_mapping: {
          charge_id: '${output.charge_id}',
        },
        timeout_ms: 60000,
      },
    },

    // 2. Fraud gate. Rules are evaluated in order against the flow context;
    //    the first match routes to its child. If NO rule matches (normal
    //    orders) the whole container is skipped.
    //    `input.flagged` is null-safe: absent -> null -> rule is false.
    {
      id: 'fraud_gate',
      node_type: 'raisin:FlowContainer',
      container_type: 'or',
      rules: [
        { condition: 'input.total > 1000', next_step: 'fraud_review' },
        { condition: 'input.flagged == true', next_step: 'fraud_review' },
      ],
      children: [
        {
          id: 'fraud_review',
          node_type: 'raisin:FlowStep',
          properties: {
            action: 'Fraud review for order {{ input.order_id }} ({{ input.total }} CHF)',
            step_type: 'human_task',
            task_type: 'review',
            assignee: '/users/admin',
            task_description:
              'Order {{ input.order_id }} needs a fraud review before fulfillment. ' +
              'Charge {{ steps.charge_payment.charge_id }} over {{ input.total }} CHF. ' +
              'Release to fulfill, cancel to void the charge.',
            priority: 5,
            options: [
              { value: 'release', label: 'Release order', style: 'success' },
              { value: 'cancel', label: 'Cancel order', style: 'danger' },
            ],
          },
        },
      ],
    },

    // 3. Routing gate after the (optional) review. The human task's
    //    response is recorded as the fraud_review step's OUTPUT, so
    //    `steps.fraud_review.action` is "cancel"/"release" after a review
    //    and null (null-safe dot access) when the review never ran.
    //    Rule order matters: cancel wins, otherwise the constant-true rule
    //    routes into fulfillment.
    {
      id: 'routing_gate',
      node_type: 'raisin:FlowContainer',
      container_type: 'or',
      rules: [
        { condition: 'steps.fraud_review.action == "cancel"', next_step: 'cancel_refund' },
        { condition: 'true', next_step: 'fulfill' },
      ],
      children: [
        // Cancel path: void the charge (forward use of refund-payment -
        // NOT the saga; the flow still completes successfully).
        {
          id: 'cancel_refund',
          node_type: 'raisin:FlowStep',
          properties: {
            action: 'Void charge {{ steps.charge_payment.charge_id }} after fraud cancel',
            function_ref: '/lib/ecommerce/refund-payment',
            arguments: {
              charge_id: '${steps.charge_payment.charge_id}',
              reason: 'order cancelled in fraud review',
            },
            timeout_ms: 60000,
          },
        },

        // Fulfillment path: allocate stock, then ship.
        {
          id: 'fulfill',
          node_type: 'raisin:FlowContainer',
          container_type: 'and',
          children: [
            {
              id: 'allocate_stock',
              node_type: 'raisin:FlowStep',
              properties: {
                action: 'Allocate stock for order {{ input.order_id }}',
                function_ref: '/lib/ecommerce/allocate-stock',
                arguments: {
                  order_id: '{{ input.order_id }}',
                  charge_id: '${steps.charge_payment.charge_id}',
                  items: '${input.items}', // whole-string expression: the ARRAY passes through intact
                },
                compensation_ref: '/lib/ecommerce/release-stock',
                compensation_input_mapping: {
                  allocation_id: '${output.allocation_id}',
                },
                timeout_ms: 60000,
              },
            },

            // PITFALL (two independent retry layers): the queued
            // function-execution JOB retries failures 3 times on a fixed
            // ~10s/30s backoff regardless of the step's retry config, and
            // then the FLOW would retry the whole step unless
            // retry_strategy 'none' disables it. timeout_ms is the flow's
            // wait deadline - it must outlive the job retry schedule
            // (~40s), otherwise the wait expires first, the flow fails via
            // timeout, and saga compensation never runs.
            {
              id: 'ship_order',
              node_type: 'raisin:FlowStep',
              properties: {
                action: 'Ship order {{ input.order_id }} to {{ input.address }}',
                function_ref: '/lib/ecommerce/ship-order',
                arguments: {
                  order_id: '{{ input.order_id }}',
                  allocation_id: '${steps.allocate_stock.allocation_id}',
                  address: '{{ input.address }}',
                  fail: '${input.simulate_carrier_outage}', // null/false in normal runs
                },
                retry_strategy: 'none',
                timeout_ms: 120000,
              },
            },
          ],
        },
      ],
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
 *   /lib/ecommerce/<name>           raisin:Function (metadata, entry_file)
 *   /lib/ecommerce/<name>/index.js  raisin:Asset with the source in the
 *                                   inline `code` property (the same place
 *                                   the Functions IDE and packages put it)
 */
async function deployFunction(client, name, title) {
  const code = await readFile(new URL(`./functions/${name}.js`, import.meta.url), 'utf8');
  await ensureNode(client, '/lib/ecommerce', name, 'raisin:Function', {
    name,
    title,
    description: `${title} (ecommerce-order example)`,
    enabled: true,
    language: 'javascript',
    execution_mode: 'async',
    entry_file: 'index.js:handler',
    version: 1,
  });
  await ensureNode(client, `/lib/ecommerce/${name}`, 'index.js', 'raisin:Asset', {
    title: 'index.js',
    file: '', // raisin:Asset requires 'file'; the source lives in the inline 'code' property
    code,
  });
  console.log(`✅ Function deployed: /lib/ecommerce/${name}`);
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
    `/api/repository/${REPO}/main/head/functions/lib/ecommerce/${functionName}/index.js`,
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
  await ensureNode(client, '/lib', 'ecommerce', 'raisin:Folder', {});
  await ensureNode(client, '/', 'flows', 'raisin:Folder', {});

  // Functions
  await deployFunction(client, 'charge-payment', 'Charge Payment');
  await deployFunction(client, 'allocate-stock', 'Allocate Stock');
  await deployFunction(client, 'ship-order', 'Ship Order');
  await deployFunction(client, 'release-stock', 'Release Stock');
  await deployFunction(client, 'refund-payment', 'Refund Payment');

  // Prove a function actually EXECUTES before relying on the flow
  const { via, out: smoke } = await smokeTestFunction(client, 'charge-payment', {
    order_id: 'SMOKE-TEST',
    amount: 12.5,
  });
  assert(
    smoke && smoke.amount === 12.5 && typeof smoke.charge_id === 'string',
    `charge-payment smoke test returned ${JSON.stringify(smoke)}`,
  );
  console.log(
    `✅ Function smoke test passed via ${via} (charge ${smoke.charge_id}, 12.5 CHF)`,
  );

  // The flow node
  const state = await ensureNode(client, '/flows', 'fulfill-order', 'raisin:Flow', {
    name: 'fulfill-order',
    title: 'Fulfill Order',
    description:
      'Charge, fraud-gate, allocate and ship an ecommerce order with saga rollback.',
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
  throw new Error('Fraud review task did not appear in the inbox');
}

function stepOutputs(status) {
  return status.variables?.step_outputs ?? {};
}

async function readInstanceNode(client, instanceId) {
  const res = await api(
    client,
    'GET',
    `/api/repository/${REPO}/main/head/raisin:system/flows/instances/${instanceId}`,
  );
  if (!res.ok) return null;
  return res.json();
}

/**
 * Complete an inbox task and wait for the flow to reach a wanted status,
 * with the known-race recovery: a job worker can grab the queued resume job
 * before its JobDataStore context is written ("Missing job context" in the
 * server log) - the job is then dropped and the flow stays 'waiting'
 * forever. If that happens, re-issue the resume via the public resume
 * endpoint (it feeds the same resume path as inbox completion).
 */
async function completeTaskAndWait(flows, inbox, instanceId, taskId, response, wanted) {
  const result = await inbox.completeTask(taskId, response);
  console.log(`✅ Task completed (${response.action}), flow resuming (job`, result.flow?.job_id + ')');
  try {
    return await waitForStatus(flows, instanceId, wanted, 20);
  } catch {
    console.log(
      '⚠️  flow still waiting after task completion (known engine race: resume ' +
        'job dropped on "Missing job context") - re-issuing resume',
    );
    await flows.resume(instanceId, { ...response, completed_by: 'admin' });
    return waitForStatus(flows, instanceId, wanted);
  }
}

// ---------------------------------------------------------------------------
// Scenario A: normal order - fraud gate skipped, charged, allocated, shipped
// ---------------------------------------------------------------------------

async function runNormalOrder(flows) {
  console.log('\n━━━ Scenario A: normal order (99 CHF, 2 line items) ━━━');

  const items = [
    { sku: 'SKU-RED-MUG', qty: 1 },
    { sku: 'SKU-TEE-M', qty: 2 },
  ];
  const { instance_id } = await flows.run(FLOW_PATH, {
    order_id: 'ORD-1001',
    total: 99,
    items,
    address: 'Bahnhofstrasse 1, 8001 Zurich',
  });
  console.log('✅ Flow started:', instance_id);

  // NOTE: the instance status reads 'waiting' transiently while each queued
  // function executes (wait_type=function_call), so we wait for a TERMINAL
  // status here and verify "no review happened" via the variables below.
  const status = await waitForStatus(flows, instance_id, ['completed', 'failed']);
  assert(
    status.status === 'completed',
    `normal order must complete without review, got: ${status.status} ${status.error ?? ''}`,
  );
  console.log('✅ Flow completed without pausing for fraud review');

  const outputs = stepOutputs(status);
  const charge = outputs.charge_payment ?? {};
  const alloc = outputs.allocate_stock ?? {};
  const ship = outputs.ship_order ?? {};

  assert(typeof charge.charge_id === 'string', `charge_payment must output charge_id, got ${JSON.stringify(charge)}`);
  assert(charge.amount === 99, `expected charge amount 99, got ${charge.amount}`);
  console.log(`  💳 charged: ${charge.charge_id} -> ${charge.amount} ${charge.currency}`);

  assert(
    outputs.fraud_review === undefined && status.variables?.__human_response === undefined,
    'fraud review must have been skipped for a normal order',
  );
  console.log('✅ Fraud gate was skipped (no rule matched)');

  // The charge_id must have flowed from charge_payment into allocate_stock,
  // and the items ARRAY must have survived the ${input.items} template.
  assert(typeof alloc.allocation_id === 'string', `allocate_stock must output allocation_id, got ${JSON.stringify(alloc)}`);
  assert(
    alloc.charge_id === charge.charge_id,
    `allocation must reference the charge from charge_payment (${alloc.charge_id} != ${charge.charge_id})`,
  );
  assert(
    Array.isArray(alloc.items) && alloc.items.length === 2,
    `items array must survive flow templates, got ${JSON.stringify(alloc.items)}`,
  );
  assert(
    alloc.items[0].sku === items[0].sku && alloc.items[1].qty === items[1].qty,
    'items array content must round-trip through the template unchanged',
  );
  assert(alloc.units_total === 3, `expected 3 allocated units, got ${alloc.units_total}`);
  console.log(`  📦 allocated: ${alloc.allocation_id} (${alloc.units_total} units, ${alloc.items.length} lines)`);

  assert(
    typeof ship.tracking_number === 'string' && ship.tracking_number.startsWith('TRK-'),
    `ship_order must output a tracking number, got ${JSON.stringify(ship)}`,
  );
  assert(
    ship.allocation_id === alloc.allocation_id,
    'shipment must reference the allocation from allocate_stock',
  );
  console.log(`  🚚 shipped: ${ship.tracking_number} via ${ship.carrier}`);

  assert(outputs.cancel_refund === undefined, 'cancel path must not have run');
}

// ---------------------------------------------------------------------------
// Scenario B: high-value order - fraud review -> release -> shipped
// ---------------------------------------------------------------------------

async function runHighValueOrder(flows, inbox) {
  console.log('\n━━━ Scenario B: high-value order (2500 CHF) -> review -> release ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    order_id: 'ORD-2002',
    total: 2500,
    items: [{ sku: 'SKU-ESPRESSO-MACHINE', qty: 1 }],
    address: 'Paradeplatz 8, 8001 Zurich',
  });
  console.log('✅ Flow started:', instance_id);

  // 'waiting' alone is ambiguous (function executions also wait briefly),
  // so the definitive pause signal is the review task showing up in the
  // assignee's inbox. Once it exists, the instance must be 'waiting'.
  const task = await findTask(inbox, instance_id);
  console.log(`  📥 inbox task: "${task.title}" [${task.task_type}, P${task.priority}]`);
  assert(task.task_type === 'review', `expected a review task, got ${task.task_type}`);

  const waiting = await flows.getInstanceStatus(instance_id);
  assert(
    waiting.status === 'waiting',
    `high-value order must pause for review, got: ${waiting.status} ${waiting.error ?? ''}`,
  );
  console.log('✅ Flow is waiting for fraud review');

  const final = await completeTaskAndWait(
    flows,
    inbox,
    instance_id,
    task.id,
    { action: 'release', comment: 'Verified with the customer by phone' },
    ['completed', 'failed'],
  );
  assert(
    final.status === 'completed',
    `released order must complete, got: ${final.status} ${final.error ?? ''}`,
  );

  const outputs = stepOutputs(final);
  const charge = outputs.charge_payment ?? {};
  const ship = outputs.ship_order ?? {};
  const decision = final.variables?.__human_response ?? {};

  assert(charge.amount === 2500, `expected charge amount 2500, got ${charge.amount}`);
  assert(
    decision.action === 'release',
    `expected __human_response.action release, got ${JSON.stringify(decision)}`,
  );
  console.log(`  👤 decision: ${decision.action} by ${decision.completed_by}`);

  assert(
    (outputs.fraud_review ?? {}).action === 'release',
    `review response must be the fraud_review step output, got ${JSON.stringify(outputs.fraud_review)}`,
  );

  assert(
    typeof ship.tracking_number === 'string',
    `released order must ship, got ${JSON.stringify(ship)}`,
  );
  assert(
    (outputs.allocate_stock ?? {}).charge_id === charge.charge_id,
    'allocation must reference the charge from charge_payment',
  );
  assert(outputs.cancel_refund === undefined, 'cancel path must not have run on release');
  console.log(`  🚚 shipped after release: ${ship.tracking_number}`);
  console.log('✅ Flow completed after fraud release');
}

// ---------------------------------------------------------------------------
// Scenario C: carrier outage - saga rollback, BOTH compensations, LIFO order
// ---------------------------------------------------------------------------

async function runCarrierOutage(flows, client) {
  console.log('\n━━━ Scenario C: carrier outage -> saga rollback (2 compensations, LIFO) ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    order_id: 'ORD-3003',
    total: 49,
    items: [{ sku: 'SKU-SOCKS', qty: 1 }],
    address: 'Langstrasse 100, 8004 Zurich',
    simulate_carrier_outage: true, // ship-order throws after charge + allocation succeeded
  });
  console.log('✅ Flow started:', instance_id);

  // The failing function is retried by the job system (~40s total) before
  // the flow sees the final failure and rolls back. While we wait, poll the
  // instance node frequently: the engine SAVES the instance after each
  // compensation, and the compensation functions take ~2s each, so the
  // intermediate stack state (release-stock executed, refund-payment still
  // pending) is observable live - that is the LIFO proof.
  console.log('  ⏳ waiting for the job retries to exhaust (~40s), polling the saga stack ...');

  const findEntry = (stack, stepId) => stack.find((e) => e.step_id === stepId);
  const statusOf = (entry) => entry?.compensation_status?.status ?? 'absent';
  let sawReleaseFirst = false; // release-stock executed while refund-payment still pending
  let finalNode = null;

  const deadline = Date.now() + 180000;
  while (Date.now() < deadline) {
    const node = await readInstanceNode(client, instance_id);
    if (node) {
      const props = node.properties ?? {};
      const stack = props.compensation_stack ?? [];
      const chargeEntry = findEntry(stack, 'charge_payment');
      const allocEntry = findEntry(stack, 'allocate_stock');

      // LIFO evidence (intermediate state persisted between compensations)
      if (statusOf(allocEntry) === 'executed' && statusOf(chargeEntry) === 'pending') {
        if (!sawReleaseFirst) {
          console.log(
            '  👀 observed live: release-stock EXECUTED while refund-payment still PENDING (LIFO)',
          );
        }
        sawReleaseFirst = true;
      }
      // Anti-LIFO evidence would disprove the ordering immediately
      assert(
        !(statusOf(chargeEntry) === 'executed' && statusOf(allocEntry) === 'pending'),
        'ANTI-LIFO state observed: refund-payment executed before release-stock',
      );

      const status = props.status;
      const stackDone = stack.length > 0 && stack.every((e) => statusOf(e) !== 'pending');
      if (status === 'rolled_back' || status === 'completed' || (status === 'failed' && stackDone)) {
        finalNode = node;
        break;
      }
    }
    await sleep(150);
  }
  assert(finalNode, 'timed out waiting for the rollback to finish');

  const props = finalNode.properties ?? {};
  assert(
    props.status === 'rolled_back',
    `expected saga rollback, got status ${props.status} (error: ${props.error ?? '-'})`,
  );
  console.log(`✅ Flow rolled back (error: ${props.error ?? 'simulated carrier outage'})`);

  const outputs = props.variables?.step_outputs ?? {};
  const charge = outputs.charge_payment ?? {};
  const alloc = outputs.allocate_stock ?? {};
  assert(typeof charge.charge_id === 'string', 'charge_payment must have succeeded before the rollback');
  assert(typeof alloc.allocation_id === 'string', 'allocate_stock must have succeeded before the rollback');
  assert(outputs.ship_order === undefined, 'ship_order must NOT have produced output');

  // --- The key assertions: TWO compensations, LIFO, correctly mapped inputs
  const stack = props.compensation_stack ?? [];
  assert(stack.length === 2, `expected 2 compensation entries, got ${JSON.stringify(stack)}`);

  // Push order mirrors forward completion order: charge first, alloc second.
  assert(
    stack[0].step_id === 'charge_payment' && stack[1].step_id === 'allocate_stock',
    `unexpected stack order: ${stack.map((e) => e.step_id).join(', ')}`,
  );

  const chargeEntry = stack[0];
  const allocEntry = stack[1];

  assert(
    allocEntry.compensation_fn === '/lib/ecommerce/release-stock' &&
      statusOf(allocEntry) === 'executed',
    `release-stock must be executed, got ${JSON.stringify(allocEntry)}`,
  );
  assert(
    allocEntry.compensation_input?.allocation_id === alloc.allocation_id,
    'compensation_input_mapping must pass ${output.allocation_id} to release-stock',
  );
  console.log(
    `  ↩️  compensation 1 (LIFO first): ${allocEntry.compensation_fn} ` +
      `({ allocation_id: ${allocEntry.compensation_input.allocation_id} })`,
  );

  assert(
    chargeEntry.compensation_fn === '/lib/ecommerce/refund-payment' &&
      statusOf(chargeEntry) === 'executed',
    `refund-payment must be executed, got ${JSON.stringify(chargeEntry)}`,
  );
  assert(
    chargeEntry.compensation_input?.charge_id === charge.charge_id,
    'compensation_input_mapping must pass ${output.charge_id} to refund-payment',
  );
  console.log(
    `  ↩️  compensation 2 (LIFO last):  ${chargeEntry.compensation_fn} ` +
      `({ charge_id: ${chargeEntry.compensation_input.charge_id} })`,
  );

  // The live observation is the actual LIFO proof (the final stack alone
  // only shows both executed, not in which order).
  assert(
    sawReleaseFirst,
    'never observed the intermediate stack state proving release-stock ran before refund-payment',
  );
  console.log('✅ LIFO rollback verified live: release-stock FIRST, then refund-payment');
}

// ---------------------------------------------------------------------------
// Scenario D: fraud review -> cancel: charge voided, fulfillment skipped
// ---------------------------------------------------------------------------

async function runFraudCancel(flows, inbox) {
  console.log('\n━━━ Scenario D: flagged order -> review -> cancel (no fulfillment) ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    order_id: 'ORD-4004',
    total: 450,
    flagged: true, // exercises the second fraud rule (total is below 1000)
    items: [{ sku: 'SKU-GIFTCARD-400', qty: 1 }],
    address: 'Dropship Depot 7, 9999 Nowhere',
  });
  console.log('✅ Flow started:', instance_id);

  const task = await findTask(inbox, instance_id);
  console.log(`  📥 inbox task: "${task.title}" [${task.task_type}, P${task.priority}]`);

  const final = await completeTaskAndWait(
    flows,
    inbox,
    instance_id,
    task.id,
    { action: 'cancel', comment: 'Stolen card pattern - cancelling' },
    ['completed', 'failed', 'rolled_back'],
  );
  assert(
    final.status === 'completed',
    `cancelled order should complete via the cancel path, got: ${final.status} ${final.error ?? ''}`,
  );

  const outputs = stepOutputs(final);
  const charge = outputs.charge_payment ?? {};
  const decision = final.variables?.__human_response ?? {};

  assert(
    decision.action === 'cancel',
    `expected __human_response.action cancel, got ${JSON.stringify(decision)}`,
  );
  console.log(`  👤 decision: ${decision.action} by ${decision.completed_by}`);

  assert(
    (outputs.fraud_review ?? {}).action === 'cancel',
    `cancel response must be the fraud_review step output, got ${JSON.stringify(outputs.fraud_review)}`,
  );

  // Fulfillment must have been skipped entirely
  assert(outputs.allocate_stock === undefined, 'allocate_stock must NOT run after cancel');
  assert(outputs.ship_order === undefined, 'ship_order must NOT run after cancel');
  console.log('✅ Fulfillment skipped (no allocation, no shipment)');

  // ... and the charge voided via the forward refund step
  const refund = outputs.cancel_refund ?? {};
  assert(
    refund.refunded === true && refund.charge_id === charge.charge_id,
    `cancel_refund must void the original charge, got ${JSON.stringify(refund)}`,
  );
  console.log(`  💳 charge voided: ${refund.charge_id} (${refund.reason})`);
}

// ---------------------------------------------------------------------------

async function main() {
  const client = new RaisinHttpClient(BASE_URL, { tenantId: 'default' });
  await client.authenticate({ username: USERNAME, password: PASSWORD });
  console.log('✅ Authenticated as', USERNAME);

  await ensureSetup(client);

  const flows = FlowClient.fromHttpClient(client, BASE_URL, REPO);
  const inbox = new InboxApi(BASE_URL, REPO, client.getAuthManager());

  await runNormalOrder(flows);
  await runHighValueOrder(flows, inbox);
  await runCarrierOutage(flows, client);
  await runFraudCancel(flows, inbox);

  console.log('\n🎉 All scenarios passed.');
}

main().catch((err) => {
  console.error('❌', err.message ?? err);
  process.exit(1);
});
