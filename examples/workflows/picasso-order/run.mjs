#!/usr/bin/env node
/**
 * Picasso/MTeX quote -> order -> supplier workflow - end-to-end demo with
 * the @raisindb/client SDK. Implements the MTeX BPMN swimlane
 * ("Quote -> Order -> Supplier Workflow", CUSTOMER / MTeX TEAM / SUPPLIER).
 *
 * What it shows:
 *   1. JS functions deployed as raisin:Function nodes (code in a child
 *      index.js raisin:Asset node) under /lib/picasso/
 *   2. A designer-format flow /flows/quote-to-order with THREE chained
 *      human tasks:
 *        check-feasibility       - function step (quote pricing)
 *        quote-review            - human approval #1 (send / decline)
 *        quote-gate              - OR container: only "send" proceeds
 *          order-approval        - human approval #2
 *          select-supplier       - human INPUT task (structured JSON)
 *          prepare-supplier-email- function step: blind drop-ship email,
 *                                  customer details REDACTED + self-audit
 *          mark-shipped          - function step: Pending -> In Transit
 *   3. Three live scenarios:
 *        (a) happy path through all three waits -> completed, privacy
 *            rule asserted on the supplier email
 *        (b) quote declined -> flow ends, NO second inbox task
 *        (c) input-task round-trip: submitted JSON lands verbatim in
 *            steps["select-supplier"] / __human_response
 *
 * Prereqs: a running raisin-server (dev mode) on RAISIN_URL.
 *
 * Run:
 *   npm install && RAISIN_URL=http://127.0.0.1:8100 node run.mjs
 */

import { readFile } from 'node:fs/promises';
import { RaisinHttpClient, FlowClient, InboxApi } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'picasso-demo';
const USERNAME = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const FLOW_PATH = '/flows/quote-to-order';

// The workflow definition in DESIGNER format - the same format the admin
// console's visual flow designer reads and writes.
//
// PITFALL (hyphenated step ids): raisin-rel identifiers only allow
// [A-Za-z0-9_], so `steps.check-feasibility.total_price` parses as a
// SUBTRACTION (steps.check - feasibility.total_price) and fails. Always
// reference hyphenated step ids with bracket indexing:
// `steps["check-feasibility"].total_price` - in {{ }} templates, ${ }
// expressions AND container rule conditions alike.
const workflowData = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    // PHASE 1 (QUOTE, MTeX lane): review request + check feasibility.
    {
      id: 'check-feasibility',
      node_type: 'raisin:FlowStep',
      properties: {
        action:
          'Check feasibility: {{ input.quantity }}x {{ input.product }} ({{ input.tier }} tier)',
        function_ref: '/lib/picasso/check-feasibility',
        arguments: {
          product: '{{ input.product }}',
          quantity: '${input.quantity}', // whole-string expression keeps the number type
          tier: '{{ input.tier }}',
        },
        retry_strategy: 'none',
        // Flow wait deadline must outlive the job retry schedule (~40s),
        // see the engine pitfalls in the README.
        timeout_ms: 120000,
      },
    },

    // PHASE 1 (QUOTE, MTeX lane): human decides to send the quote.
    {
      id: 'quote-review',
      node_type: 'raisin:FlowStep',
      properties: {
        action:
          'Send quote: {{ input.quantity }}x {{ input.product }} ' +
          '({{ steps["check-feasibility"].total_price }} CHF)?',
        step_type: 'human_task',
        task_type: 'approval',
        assignee: '/users/admin',
        task_description:
          'Quote request from {{ input.customer.company }} ({{ input.tier }} tier). ' +
          'Feasible: {{ steps["check-feasibility"].feasible }}, ' +
          'unit price {{ steps["check-feasibility"].unit_price }} CHF, ' +
          'total {{ steps["check-feasibility"].total_price }} CHF.',
        priority: 3,
        options: [
          { value: 'send', label: 'Send quote', style: 'success' },
          { value: 'decline', label: 'Decline request', style: 'danger' },
        ],
      },
    },

    // Decline gate: only a sent (and implicitly customer-accepted) quote
    // becomes an order. If the rule does not match (action == "decline")
    // the whole container is skipped and the flow completes - no
    // order-approval task is ever created.
    {
      id: 'quote-gate',
      node_type: 'raisin:FlowContainer',
      container_type: 'or',
      rules: [
        {
          condition: 'steps["quote-review"].action == "send"',
          next_step: 'order-pipeline',
        },
      ],
      children: [
        // AND container: the whole order + supplier pipeline runs
        // sequentially once the quote was sent.
        {
          id: 'order-pipeline',
          node_type: 'raisin:FlowContainer',
          container_type: 'and',
          children: [
            // PHASE 2 (ORDER, MTeX lane): receive order, review details,
            // approve (human feasibility check). Second wait/resume cycle.
            {
              id: 'order-approval',
              node_type: 'raisin:FlowStep',
              properties: {
                action: 'Approve order for {{ input.customer.company }}?',
                step_type: 'human_task',
                task_type: 'approval',
                assignee: '/users/admin',
                task_description:
                  'Order form submitted by {{ input.customer.company }} ' +
                  '({{ input.tier }} tier): {{ input.quantity }}x {{ input.product }}, ' +
                  'quoted total {{ steps["check-feasibility"].total_price }} CHF. ' +
                  'MTeX reviews each order before any supplier contact.',
                priority: 4,
                options: [
                  { value: 'approve', label: 'Approve order', style: 'success' },
                  { value: 'reject', label: 'Reject order', style: 'danger' },
                ],
              },
            },

            // PHASE 3 (SUPPLIER, MTeX lane): select supplier - HITL
            // decision as an INPUT task (structured JSON, not buttons).
            {
              id: 'select-supplier',
              node_type: 'raisin:FlowStep',
              properties: {
                action: 'Select supplier for {{ input.quantity }}x {{ input.product }}',
                step_type: 'human_task',
                task_type: 'input',
                assignee: '/users/admin',
                task_description:
                  'Pick the supplier and shipping mode for the approved order ' +
                  '(quoted {{ steps["check-feasibility"].total_price }} CHF). ' +
                  'The supplier will NOT see any customer details.',
                priority: 3,
                input_schema: {
                  type: 'object',
                  properties: {
                    supplier: { type: 'string', description: 'Supplier company name' },
                    shipping_mode: {
                      type: 'string',
                      enum: ['direct', 'via_mtex'],
                      description: 'Blind drop-ship direct, or via the MTeX warehouse',
                    },
                  },
                  required: ['supplier', 'shipping_mode'],
                },
              },
            },

            // PHASE 3 (SUPPLIER): prepare the blind order email. Receives
            // the FULL customer object and must redact every field.
            {
              id: 'prepare-supplier-email',
              node_type: 'raisin:FlowStep',
              properties: {
                action: 'Prepare redacted order email for {{ steps["select-supplier"].supplier }}',
                function_ref: '/lib/picasso/prepare-supplier-email',
                arguments: {
                  customer: '${input.customer}', // full object - redacted inside
                  product: '{{ input.product }}',
                  quantity: '${input.quantity}',
                  total_price: '${steps["check-feasibility"].total_price}',
                  supplier: '{{ steps["select-supplier"].supplier }}',
                  shipping_mode: '{{ steps["select-supplier"].shipping_mode }}',
                },
                retry_strategy: 'none',
                timeout_ms: 120000,
              },
            },

            // PHASE 3 (SUPPLIER): supplier ships, status Pending -> In Transit.
            {
              id: 'mark-shipped',
              node_type: 'raisin:FlowStep',
              properties: {
                action: 'Mark shipped: {{ steps["prepare-supplier-email"].supplier_order_ref }}',
                function_ref: '/lib/picasso/mark-shipped',
                arguments: {
                  supplier_order_ref: '${steps["prepare-supplier-email"].supplier_order_ref}',
                  supplier: '{{ steps["select-supplier"].supplier }}',
                  shipping_mode: '{{ steps["select-supplier"].shipping_mode }}',
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

const FUNCTIONS = [
  ['check-feasibility', 'Check Feasibility'],
  ['prepare-supplier-email', 'Prepare Supplier Email'],
  ['mark-shipped', 'Mark Shipped'],
];

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
 *   /lib/picasso/<name>           raisin:Function (metadata, entry_file)
 *   /lib/picasso/<name>/index.js  raisin:Asset with the source in the
 *                                 inline `code` property
 */
async function deployFunction(client, name, title) {
  const code = await readFile(new URL(`./functions/${name}.js`, import.meta.url), 'utf8');
  await ensureNode(client, '/lib/picasso', name, 'raisin:Function', {
    name,
    title,
    description: `${title} (picasso-order example)`,
    enabled: true,
    language: 'javascript',
    execution_mode: 'async',
    entry_file: 'index.js:handler',
    version: 1,
  });
  await ensureNode(client, `/lib/picasso/${name}`, 'index.js', 'raisin:Asset', {
    title: 'index.js',
    file: '', // raisin:Asset requires 'file'; the source lives in the inline 'code' property
    code,
  });
  console.log(`✅ Function deployed: /lib/picasso/${name}`);
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
    const err = new Error(`invoke ${name} failed: HTTP ${res.status} ${JSON.stringify(body)}`);
    err.status = res.status;
    throw err;
  }
  const raw = body.result ?? {};
  return raw && typeof raw === 'object' && 'result' in raw && 'success' in raw
    ? raw.result
    : raw;
}

/**
 * Fallback smoke test via POST /api/files/{repo}/run (direct file
 * execution, SSE response). Needed because of a current engine bug: the
 * /api/functions lookup builds its node service WITHOUT the caller's auth
 * context, so RLS denies every function node and invoke 404s even for
 * admins. Flow execution is unaffected.
 */
async function runFileDirect(client, functionName, input) {
  const fileRes = await api(
    client,
    'GET',
    `/api/repository/${REPO}/main/head/functions/lib/picasso/${functionName}/index.js`,
  );
  if (!fileRes.ok) throw new Error(`could not load index.js node for ${functionName}`);
  const { id: nodeId } = await fileRes.json();

  const res = await api(client, 'POST', `/api/files/${REPO}/run`, {
    node_id: nodeId,
    handler: 'handler',
    input,
  });
  if (!res.ok) throw new Error(`files/run failed: HTTP ${res.status} ${await res.text()}`);

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
  await ensureNode(client, '/lib', 'picasso', 'raisin:Folder', {});
  await ensureNode(client, '/', 'flows', 'raisin:Folder', {});

  // Functions
  for (const [name, title] of FUNCTIONS) {
    await deployFunction(client, name, title);
  }

  // Prove the functions actually EXECUTE before relying on the flow.
  const { via, out: feas } = await smokeTestFunction(client, 'check-feasibility', {
    product: 'uv-dtf-roll',
    quantity: 10,
    tier: 'starter',
  });
  assert(
    feas && feas.feasible === true && feas.total_price === 800,
    `check-feasibility smoke test returned ${JSON.stringify(feas)}`,
  );
  console.log(`✅ check-feasibility smoke test passed via ${via} (10 rolls -> 800 CHF)`);

  // The privacy function must redact even in isolation.
  const { out: mail } = await smokeTestFunction(client, 'prepare-supplier-email', {
    customer: {
      company: 'Smoke AG',
      contact_name: 'Sam Smoke',
      email: 'sam@smoke.ch',
      po_number: 'PO-SMOKE-1',
    },
    product: 'uv-dtf-roll',
    quantity: 10,
    total_price: 800,
    supplier: 'UV-Print AG',
    shipping_mode: 'via_mtex',
  });
  assert(
    mail &&
      typeof mail.email_body === 'string' &&
      Object.values(mail.redaction_check).every((v) => v === false),
    `prepare-supplier-email smoke test returned ${JSON.stringify(mail)}`,
  );
  console.log('✅ prepare-supplier-email smoke test passed (redaction self-audit clean)');

  // The flow node
  const state = await ensureNode(client, '/flows', 'quote-to-order', 'raisin:Flow', {
    name: 'quote-to-order',
    title: 'Quote to Order (Picasso/MTeX)',
    description:
      'MTeX quote -> order -> supplier pipeline: feasibility check, quote approval, ' +
      'order approval, HITL supplier selection, blind drop-ship email, shipping.',
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

/** Find the pending inbox task a given step created for an instance. */
async function findTask(inbox, instanceId, stepId, attempts = 40) {
  for (let i = 0; i < attempts; i++) {
    const { tasks } = await inbox.listTasks({
      status: 'pending',
      assignee: '/users/admin',
    });
    const task = tasks.find(
      (t) => t.flow_instance_id === instanceId && (!stepId || t.step_id === stepId),
    );
    if (task) return task;
    await sleep(500);
  }
  throw new Error(`Task for step '${stepId}' did not appear in the inbox (${instanceId})`);
}

/** Read the raw flow instance node (raisin:system workspace). */
async function getInstanceNode(client, instanceId) {
  const res = await api(
    client,
    'GET',
    `/api/repository/${REPO}/main/head/raisin:system/flows/instances/${instanceId}`,
  );
  if (!res.ok) throw new Error(`could not read instance node: HTTP ${res.status}`);
  return res.json();
}

/**
 * Rebuild each function step's input exactly like the flow's argument
 * mapping would, from the original flow input + the step outputs so far.
 * Used ONLY by the dropped-resume recovery below.
 */
const FUNCTION_STEP_INPUTS = {
  'check-feasibility': (input) => ({
    product: input.product,
    quantity: input.quantity,
    tier: input.tier,
  }),
  'prepare-supplier-email': (input, steps) => ({
    customer: input.customer,
    product: input.product,
    quantity: input.quantity,
    total_price: steps['check-feasibility']?.total_price,
    supplier: steps['select-supplier']?.supplier,
    shipping_mode: steps['select-supplier']?.shipping_mode,
  }),
  'mark-shipped': (input, steps) => ({
    supplier_order_ref: steps['prepare-supplier-email']?.supplier_order_ref,
    supplier: steps['select-supplier']?.supplier,
    shipping_mode: steps['select-supplier']?.shipping_mode,
  }),
};

/**
 * RECOVERY for the dropped-resume engine race ("Missing job context"):
 * `register_job()` makes a job visible to workers before JobDataStore.put()
 * writes its context; a worker that wins the race drops the job WITHOUT
 * retry. This demo observed it dropping BOTH kinds of flow-resume jobs:
 *   - the resume after an inbox-task completion (flow stuck 'waiting' at
 *     the human task) -> re-issue the resume with the same response, and
 *   - the resume carrying a FUNCTION step's result (the function job
 *     itself completed fine, the flow stays 'waiting' at the function
 *     step forever) -> for a function_call wait the engine expects the
 *     resume payload in job-result shape `{ success, result }`
 *     (process_resume_data stores it as __function_result verbatim), so
 *     re-run the (idempotent) function via /api/files/run and resume
 *     with `{ success: true, result }`.
 *
 * Returns true if a recovery resume was issued.
 */
async function recoverIfStuck({ client, flows }, instanceId, flowInput, completedHumanStep) {
  const node = await getInstanceNode(client, instanceId);
  const status = node.properties?.status;
  const current = node.properties?.current_node_id;
  if (status !== 'waiting') return false;

  if (completedHumanStep && current === completedHumanStep.stepId) {
    console.log(
      `⚠️  flow still waiting at human task '${current}' after completion (known ` +
        `engine race: resume job dropped on "Missing job context") - re-issuing resume`,
    );
    await flows.resume(instanceId, {
      ...completedHumanStep.payload,
      completed_by: USERNAME,
    });
    return true;
  }

  const buildInput = FUNCTION_STEP_INPUTS[current];
  if (buildInput) {
    console.log(
      `⚠️  flow stuck waiting at function step '${current}' (known engine race: the ` +
        `flow-resume job carrying the function result was dropped on "Missing job ` +
        `context") - re-running the function and resuming with { success, result }`,
    );
    const steps = node.properties?.variables?.step_outputs ?? {};
    const result = await runFileDirect(client, current, buildInput(flowInput, steps));
    await flows.resume(instanceId, { success: true, result });
    return true;
  }

  console.log(`  ⏳ flow waiting at '${current}' - no recovery applicable, waiting longer`);
  return false;
}

/**
 * Complete an inbox task, retrying on the two completion races.
 *
 * NEW PITFALLS (both observed live; the inbox task becomes LISTABLE
 * before it is actually completable):
 *  1. "Inbox task '<id>' not found" - the task shows up in the inbox
 *     list a moment before it is readable by id (load_task_node
 *     get-by-id misses while the list scan already sees it).
 *  2. "Invalid state transition from running to resumed" - the human
 *     task step creates the inbox task BEFORE the flow persists its
 *     'waiting' status; complete_task validates the instance is Waiting
 *     and rejects a completion that lands in that gap.
 * Both are transient (milliseconds) - retry briefly.
 */
async function completeTaskWithRetry(inbox, taskId, payload, attempts = 10) {
  for (let i = 0; ; i++) {
    try {
      return await inbox.completeTask(taskId, payload);
    } catch (err) {
      const msg = String(err?.message ?? err);
      const retryable =
        /not found/i.test(msg) || /invalid state transition from running/i.test(msg);
      if (i < attempts - 1 && retryable) {
        console.log(`  ⏳ task not completable yet (${msg.trim()}) - retrying complete`);
        await sleep(500);
        continue;
      }
      throw err;
    }
  }
}

/**
 * Complete an inbox task and wait until the flow has actually MOVED ON
 * (next human task created, or a terminal status), recovering from
 * dropped resume jobs along the way.
 *
 * IMPORTANT with CHAINED human tasks: never blindly re-resume. The
 * current_node_id guard in recoverIfStuck ensures a human-task re-resume
 * only fires while the flow still waits at the COMPLETED step -
 * re-resuming after it already advanced would feed the stale payload to
 * the NEXT human task's wait. Between two expectations the flow may also
 * traverse SEVERAL function steps (prepare-supplier-email then
 * mark-shipped), each with its own droppable resume job, hence the loop.
 */
async function completeTaskAndAdvance(ctx, instanceId, task, payload, expect, flowInput) {
  const { flows, inbox } = ctx;
  const result = await completeTaskWithRetry(inbox, task.id, payload);
  console.log(
    `  ✔ completed '${task.step_id}' task (resume job ${result.flow?.job_id ?? 'n/a'})`,
  );

  const advanced = async (attempts) => {
    if (expect.nextStepId) return findTask(inbox, instanceId, expect.nextStepId, attempts);
    return waitForStatus(flows, instanceId, ['completed', 'failed', 'rolled_back'], attempts);
  };

  const completedHumanStep = { stepId: task.step_id, payload };
  for (let round = 0; ; round++) {
    try {
      return await advanced(20); // ~10s per round
    } catch (err) {
      if (round >= 5) throw err;
      await recoverIfStuck(ctx, instanceId, flowInput, completedHumanStep);
    }
  }
}

function stepOutputs(status) {
  return status.variables?.step_outputs ?? {};
}

const CUSTOMER = {
  company: 'Atelier Brandt GmbH',
  contact_name: 'Mara Brandt',
  email: 'mara@atelier-brandt.ch',
  po_number: 'PO-2026-0042',
};

/** Start a quote-to-order instance and hand back the quote-review task. */
async function startInstance(ctx, input) {
  const { flows, inbox } = ctx;
  const { instance_id } = await flows.run(FLOW_PATH, input);
  console.log('✅ Flow started:', instance_id);

  // The definitive pause signal is the quote-review task in the inbox.
  // (The check-feasibility resume job can be dropped too - recover.)
  let task;
  for (let round = 0; ; round++) {
    try {
      task = await findTask(inbox, instance_id, 'quote-review', 20);
      break;
    } catch (err) {
      if (round >= 3) throw err;
      await recoverIfStuck(ctx, instance_id, input, null);
    }
  }
  console.log(`  📥 inbox task: "${task.title}" [${task.task_type}, P${task.priority}]`);
  assert(task.task_type === 'approval', `expected approval task, got ${task.task_type}`);

  const waiting = await flows.getInstanceStatus(instance_id);
  assert(
    waiting.status === 'waiting',
    `flow must pause for quote review, got: ${waiting.status} ${waiting.error ?? ''}`,
  );
  return { instanceId: instance_id, quoteTask: task };
}

// ---------------------------------------------------------------------------
// Scenario A: full happy path through all THREE waits
// ---------------------------------------------------------------------------

async function runHappyPath(ctx) {
  console.log('\n━━━ Scenario A: happy path (quote sent -> order approved -> supplier) ━━━');
  const { inbox } = ctx;

  const flowInput = {
    product: 'uv-dtf-roll',
    quantity: 25,
    tier: 'business',
    customer: CUSTOMER,
  };
  const { instanceId, quoteTask } = await startInstance(ctx, flowInput);

  // The quote task title must carry the price computed by check-feasibility
  // (resolved via steps["check-feasibility"] - bracket syntax, see pitfall).
  assert(
    quoteTask.title.includes('1900'),
    `quote task title must contain the computed total 1900, got: "${quoteTask.title}"`,
  );

  // WAIT 1 -> approve the quote; expect the order-approval task next.
  const orderTask = await completeTaskAndAdvance(
    ctx,
    instanceId,
    quoteTask,
    { action: 'send', comment: 'Pricing confirmed, quote sent to customer' },
    { nextStepId: 'order-approval' },
    flowInput,
  );
  console.log(`  📥 inbox task: "${orderTask.title}" [${orderTask.task_type}, P${orderTask.priority}]`);
  assert(orderTask.task_type === 'approval', `expected approval, got ${orderTask.task_type}`);
  assert(orderTask.priority === 4, `order-approval must be P4, got P${orderTask.priority}`);
  assert(
    orderTask.title.includes(CUSTOMER.company),
    `order task title must name the customer, got: "${orderTask.title}"`,
  );

  // WAIT 2 -> approve the order; expect the select-supplier INPUT task.
  const supplierTask = await completeTaskAndAdvance(
    ctx,
    instanceId,
    orderTask,
    { action: 'approve', comment: 'Details reviewed, feasibility confirmed' },
    { nextStepId: 'select-supplier' },
    flowInput,
  );
  console.log(`  📥 inbox task: "${supplierTask.title}" [${supplierTask.task_type}]`);
  assert(supplierTask.task_type === 'input', `expected input task, got ${supplierTask.task_type}`);
  assert(
    supplierTask.input_schema?.properties?.supplier?.type === 'string' &&
      Array.isArray(supplierTask.input_schema?.properties?.shipping_mode?.enum),
    `input task must carry the input_schema, got ${JSON.stringify(supplierTask.input_schema)}`,
  );

  // WAIT 3 -> submit the structured supplier selection; expect completion.
  const supplierChoice = { supplier: 'UV-Print AG', shipping_mode: 'via_mtex' };
  const final = await completeTaskAndAdvance(
    ctx,
    instanceId,
    supplierTask,
    supplierChoice,
    { terminal: true },
    flowInput,
  );
  assert(
    final.status === 'completed',
    `flow must complete, got: ${final.status} ${final.error ?? ''}`,
  );
  console.log('✅ Flow completed after three human interactions');

  const outputs = stepOutputs(final);
  const feas = outputs['check-feasibility'] ?? {};
  const quote = outputs['quote-review'] ?? {};
  const order = outputs['order-approval'] ?? {};
  const chosen = outputs['select-supplier'] ?? {};
  const mail = outputs['prepare-supplier-email'] ?? {};
  const shipped = outputs['mark-shipped'] ?? {};

  // Pricing: 25 x (80 * 0.95) = 25 x 76 = 1900 CHF
  assert(feas.feasible === true, 'check-feasibility must report feasible');
  assert(feas.unit_price === 76, `expected unit_price 76, got ${feas.unit_price}`);
  assert(feas.total_price === 1900, `expected total_price 1900, got ${feas.total_price}`);
  console.log(`  💰 quote: ${feas.quantity}x ${feas.product} @ ${feas.unit_price} -> ${feas.total_price} CHF`);

  // Both approvals recorded as step outputs
  assert(quote.action === 'send', `quote-review output action must be 'send', got ${quote.action}`);
  assert(order.action === 'approve', `order-approval output must be 'approve', got ${order.action}`);

  // Input task response landed in steps
  assert(chosen.supplier === 'UV-Print AG', `supplier mismatch: ${JSON.stringify(chosen)}`);
  assert(chosen.shipping_mode === 'via_mtex', `shipping_mode mismatch: ${JSON.stringify(chosen)}`);
  console.log(`  🏭 supplier selected: ${chosen.supplier} (${chosen.shipping_mode})`);

  // THE PRIVACY RULE: supplier email names the supplier, never the customer.
  const body = mail.email_body ?? '';
  assert(body.includes('UV-Print AG'), 'email body must address the supplier');
  assert(body.includes('uv-dtf-roll') && body.includes('25'), 'email must carry item + quantity');
  for (const [label, value] of [
    ['customer company', CUSTOMER.company],
    ['contact name', CUSTOMER.contact_name],
    ['customer email', CUSTOMER.email],
    ['PO number', CUSTOMER.po_number],
  ]) {
    assert(!body.includes(value), `PRIVACY VIOLATION: email body contains ${label} (${value})`);
  }
  assert(
    mail.redaction_check &&
      Object.values(mail.redaction_check).every((v) => v === false),
    `redaction_check must be all false, got ${JSON.stringify(mail.redaction_check)}`,
  );
  console.log('  🕶️  privacy rule held: no customer name/contact/email/PO in the supplier email');
  console.log(`  ✉️  ${mail.supplier_order_ref}: blind drop-ship email prepared`);

  // Shipping status update
  assert(shipped.status === 'in_transit', `expected in_transit, got ${shipped.status}`);
  assert(typeof shipped.tracking_ref === 'string' && shipped.tracking_ref.startsWith('TRK-'),
    `expected a tracking ref, got ${JSON.stringify(shipped)}`);
  console.log(`  🚚 status: pending -> ${shipped.status} (${shipped.tracking_ref} via ${shipped.carrier})`);

  // No pending task left for this instance
  const { tasks } = await inbox.listTasks({ status: 'pending', assignee: '/users/admin' });
  assert(
    !tasks.some((t) => t.flow_instance_id === instanceId),
    'no pending inbox task may remain for the completed instance',
  );
}

// ---------------------------------------------------------------------------
// Scenario B: quote declined - flow ends, no order-approval task created
// ---------------------------------------------------------------------------

async function runDeclinedQuote(ctx) {
  console.log('\n━━━ Scenario B: quote declined at quote-review ━━━');
  const { inbox } = ctx;

  const flowInput = {
    product: 'uv-phone-case',
    quantity: 500,
    tier: 'starter',
    customer: CUSTOMER,
  };
  const { instanceId, quoteTask } = await startInstance(ctx, flowInput);

  const final = await completeTaskAndAdvance(
    ctx,
    instanceId,
    quoteTask,
    { action: 'decline', comment: 'Margins too thin at this volume' },
    { terminal: true },
    flowInput,
  );
  assert(
    final.status === 'completed',
    `declined quote must end the flow cleanly, got: ${final.status} ${final.error ?? ''}`,
  );

  const decision = final.variables?.__human_response ?? {};
  assert(
    decision.action === 'decline',
    `__human_response.action must be 'decline', got ${JSON.stringify(decision)}`,
  );
  console.log(`  👤 decision recorded: ${decision.action} by ${decision.completed_by}`);

  const outputs = stepOutputs(final);
  assert(outputs['quote-review']?.action === 'decline', 'quote-review output must record decline');
  for (const stepId of ['order-approval', 'select-supplier', 'prepare-supplier-email', 'mark-shipped']) {
    assert(outputs[stepId] === undefined, `step '${stepId}' must NOT have run after a decline`);
  }
  console.log('✅ Order pipeline skipped entirely (quote-gate rule did not match)');

  // The decisive assertion: no SECOND inbox task was ever created for
  // this instance - pending or otherwise.
  for (const status of ['pending', 'completed']) {
    const { tasks } = await inbox.listTasks({ status, assignee: '/users/admin' });
    const extras = tasks.filter(
      (t) => t.flow_instance_id === instanceId && t.step_id !== 'quote-review',
    );
    assert(
      extras.length === 0,
      `no non-quote-review task may exist (${status}), found: ${extras
        .map((t) => t.step_id)
        .join(', ')}`,
    );
  }
  console.log('✅ No order-approval task was created');
}

// ---------------------------------------------------------------------------
// Scenario C: input-task round-trip - submitted JSON lands verbatim
// ---------------------------------------------------------------------------

async function runInputRoundTrip(ctx) {
  console.log('\n━━━ Scenario C: input-task round-trip (select-supplier) ━━━');

  const flowInput = {
    product: 'uv-sticker-sheet',
    quantity: 1000,
    tier: 'enterprise',
    customer: CUSTOMER,
  };
  const { instanceId, quoteTask } = await startInstance(ctx, flowInput);

  const orderTask = await completeTaskAndAdvance(
    ctx,
    instanceId,
    quoteTask,
    { action: 'send' },
    { nextStepId: 'order-approval' },
    flowInput,
  );
  const supplierTask = await completeTaskAndAdvance(
    ctx,
    instanceId,
    orderTask,
    { action: 'approve' },
    { nextStepId: 'select-supplier' },
    flowInput,
  );

  // Submit the structured input EXACTLY as the schema describes.
  const submitted = { supplier: 'Nippon UV Supply K.K.', shipping_mode: 'direct' };
  const final = await completeTaskAndAdvance(
    ctx,
    instanceId,
    supplierTask,
    submitted,
    { terminal: true },
    flowInput,
  );
  assert(final.status === 'completed', `flow must complete, got ${final.status}`);

  // Round-trip into steps["select-supplier"]: the submitted fields come
  // back VERBATIM. The engine adds exactly two metadata fields on top
  // (completed_by, task_path) - see service.rs complete_task.
  const echoed = stepOutputs(final)['select-supplier'] ?? {};
  for (const [k, v] of Object.entries(submitted)) {
    assert(echoed[k] === v, `steps round-trip mismatch for '${k}': ${JSON.stringify(echoed)}`);
  }
  assert(
    typeof echoed.completed_by === 'string' &&
      echoed.completed_by.length > 0 &&
      typeof echoed.task_path === 'string',
    `engine metadata missing on step output: ${JSON.stringify(echoed)}`,
  );
  const extraKeys = Object.keys(echoed).filter(
    (k) => !(k in submitted) && k !== 'completed_by' && k !== 'task_path',
  );
  assert(extraKeys.length === 0, `unexpected extra keys in step output: ${extraKeys.join(', ')}`);

  // ... and into __human_response (the last completed task's response).
  const hr = final.variables?.__human_response ?? {};
  for (const [k, v] of Object.entries(submitted)) {
    assert(hr[k] === v, `__human_response round-trip mismatch for '${k}': ${JSON.stringify(hr)}`);
  }
  console.log('  🔁 submitted input came back verbatim in steps["select-supplier"] AND __human_response');

  // The submitted choice drove the downstream email + shipping.
  const mail = stepOutputs(final)['prepare-supplier-email'] ?? {};
  assert(
    (mail.email_body ?? '').includes(submitted.supplier),
    'supplier from the input task must appear in the email body',
  );
  const shipped = stepOutputs(final)['mark-shipped'] ?? {};
  assert(shipped.carrier === 'DHL Express', `direct shipping must pick DHL, got ${shipped.carrier}`);
  console.log(`  🚚 direct blind drop-ship via ${shipped.carrier} (${shipped.tracking_ref})`);
  console.log('✅ Input task round-trip verified');
}

// ---------------------------------------------------------------------------

async function main() {
  const client = new RaisinHttpClient(BASE_URL, { tenantId: 'default' });
  await client.authenticate({ username: USERNAME, password: PASSWORD });
  console.log('✅ Authenticated as', USERNAME);

  await ensureSetup(client);

  const flows = FlowClient.fromHttpClient(client, BASE_URL, REPO);
  const inbox = new InboxApi(BASE_URL, REPO, client.getAuthManager());
  const ctx = { client, flows, inbox };

  await runHappyPath(ctx);
  await runDeclinedQuote(ctx);
  await runInputRoundTrip(ctx);

  console.log('\n🎉 All scenarios passed.');
}

main().catch((err) => {
  console.error('❌', err.message ?? err);
  process.exit(1);
});
