#!/usr/bin/env node
/**
 * Employee onboarding workflow - end-to-end demo with the @raisindb/client SDK.
 *
 * What it shows:
 *   1. JS functions deployed as raisin:Function nodes (code in a child
 *      index.js raisin:Asset node) under /lib/onboarding/
 *   2. A designer-format flow /flows/onboard-employee:
 *        create-accounts  - function step with saga compensation
 *                           (deprovision-accounts)
 *        equipment-gate   - OR container: a REL rule routes engineers to an
 *                           order-laptop step; everyone else skips it
 *        manager-approval - human task assigned to /users/admin (inbox)
 *        send-welcome     - function step consuming the provisioned email
 *                           AND the manager's decision
 *   3. Four live scenarios:
 *        (a) engineer    -> laptop ordered, approval, welcome email
 *        (b) contractor  -> equipment gate skipped, approval, welcome email
 *        (c) rejection   -> manager rejects, flow completes with the
 *                           rejection-notice variant of the email
 *        (d) mail outage -> send-welcome fails permanently, the saga rolls
 *                           back and deprovision-accounts compensates
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
const REPO = process.env.RAISIN_REPO ?? 'onboarding-demo';
const USERNAME = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const FLOW_PATH = '/flows/onboard-employee';

// The workflow definition in DESIGNER format - the same format the admin
// console's visual flow designer reads and writes. Execution order is the
// array order; the engine injects start/end and lowers containers.
//
// NOTE on step ids with hyphens: REL identifiers only allow [a-z0-9_], so
// `steps.create-accounts.email` would parse as a SUBTRACTION. Hyphenated
// step outputs must be referenced with bracket access:
//   ${steps['create-accounts'].email}
const workflowData = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    // 1. Provision the accounts. Registers saga compensation once it
    //    succeeds: if a LATER step fails unrecoverably, deprovision-accounts
    //    runs with the mapped input (output.* = this step's fresh output).
    {
      id: 'create-accounts',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Provision accounts for {{ input.name }} ({{ input.role }})',
        function_ref: '/lib/onboarding/create-accounts',
        arguments: {
          name: '{{ input.name }}',
          role: '{{ input.role }}',
          start_date: '{{ input.start_date }}',
        },
        compensation_ref: '/lib/onboarding/deprovision-accounts',
        compensation_input_mapping: {
          account_id: '${output.account_id}',
        },
        timeout_ms: 30000,
      },
    },

    // 2. Equipment gate. Rules are evaluated in order against the flow
    //    context; the first match routes to its child. If NO rule matches
    //    (non-engineers) the whole container is skipped.
    {
      id: 'equipment-gate',
      node_type: 'raisin:FlowContainer',
      container_type: 'or',
      rules: [{ condition: 'input.role == "engineer"', next_step: 'order-laptop' }],
      children: [
        {
          id: 'order-laptop',
          node_type: 'raisin:FlowStep',
          properties: {
            action: 'Order laptop for {{ input.name }}',
            function_ref: '/lib/onboarding/order-laptop',
            arguments: {
              account_id: "${steps['create-accounts'].account_id}",
              name: '{{ input.name }}',
              role: '{{ input.role }}',
            },
            timeout_ms: 30000,
          },
        },
      ],
    },

    // 3. Manager approval - human task in /users/admin's inbox. The flow
    //    pauses ('waiting') until the task is completed; the response
    //    becomes this step's output AND the __human_response variable.
    {
      id: 'manager-approval',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Approve onboarding for {{ input.name }} ({{ input.role }})',
        step_type: 'human_task',
        task_type: 'approval',
        assignee: '/users/admin',
        task_description:
          'New hire {{ input.name }} ({{ input.role }}) starts ' +
          "{{ input.start_date }}. Provisioned account: {{ steps['create-accounts'].email }}.",
        priority: 3,
        options: [
          { value: 'approve', label: 'Approve', style: 'success' },
          { value: 'reject', label: 'Reject', style: 'danger' },
        ],
      },
    },

    // 4. Compose the welcome email from the provisioned address and the
    //    manager's decision (rejections get the rejection-notice variant;
    //    the flow still completes - the decision is data, not control flow).
    //    PITFALL (two independent retry layers):
    //      - the queued function-execution JOB retries failures 3 times on
    //        a fixed ~10s/30s backoff regardless of the step's retry config,
    //      - then the FLOW retries the whole step (default 3 more times)
    //        unless retry_strategy 'none' disables it.
    //    timeout_ms is the flow's wait deadline - it must outlive the job
    //    retry schedule (~40s), otherwise the wait expires first, the flow
    //    fails via timeout, and saga compensation never runs.
    {
      id: 'send-welcome',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Send welcome email to {{ input.name }}',
        function_ref: '/lib/onboarding/send-welcome',
        arguments: {
          name: '{{ input.name }}',
          email: "${steps['create-accounts'].email}",
          decision: '${__human_response.action}',
          fail: '${input.simulate_welcome_failure}', // null/false in normal runs
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
 *   /lib/onboarding/<name>           raisin:Function (metadata, entry_file)
 *   /lib/onboarding/<name>/index.js  raisin:Asset with the source in the
 *                                    inline `code` property (the same place
 *                                    the Functions IDE and packages put it)
 */
async function deployFunction(client, name, title) {
  const code = await readFile(new URL(`./functions/${name}.js`, import.meta.url), 'utf8');
  await ensureNode(client, '/lib/onboarding', name, 'raisin:Function', {
    name,
    title,
    description: `${title} (employee-onboarding example)`,
    enabled: true,
    language: 'javascript',
    execution_mode: 'async',
    entry_file: 'index.js:handler',
    version: 1,
  });
  await ensureNode(client, `/lib/onboarding/${name}`, 'index.js', 'raisin:Asset', {
    title: 'index.js',
    file: '', // raisin:Asset requires 'file'; the source lives in the inline 'code' property
    code,
  });
  console.log(`✅ Function deployed: /lib/onboarding/${name}`);
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
    `/api/repository/${REPO}/main/head/functions/lib/onboarding/${functionName}/index.js`,
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
  await ensureNode(client, '/lib', 'onboarding', 'raisin:Folder', {});
  await ensureNode(client, '/', 'flows', 'raisin:Folder', {});

  // Functions
  await deployFunction(client, 'create-accounts', 'Create Accounts');
  await deployFunction(client, 'order-laptop', 'Order Laptop');
  await deployFunction(client, 'send-welcome', 'Send Welcome Email');
  await deployFunction(client, 'deprovision-accounts', 'Deprovision Accounts');

  // Prove a function actually EXECUTES before relying on the flow
  const { via, out: smoke } = await smokeTestFunction(client, 'create-accounts', {
    name: 'Smoke Test',
    role: 'contractor',
    start_date: '2026-07-01',
  });
  assert(
    smoke &&
      smoke.email === 'smoke.test@example-corp.com' &&
      typeof smoke.account_id === 'string' &&
      Array.isArray(smoke.systems) &&
      !smoke.systems.includes('github'),
    `create-accounts smoke test returned ${JSON.stringify(smoke)}`,
  );
  console.log(`✅ Function smoke test passed via ${via} (${smoke.email})`);

  // The flow node
  const state = await ensureNode(client, '/flows', 'onboard-employee', 'raisin:Flow', {
    name: 'onboard-employee',
    title: 'Onboard Employee',
    description:
      'Provision accounts, order hardware for engineers, manager approval, welcome email.',
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

async function findTask(inbox, instanceId, attempts = 60) {
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

/**
 * Wait for the approval task, verify the flow paused, complete the task,
 * then wait for one of `terminal` - with the resume-race recovery applied.
 *
 * RECOVERY for an engine race: a job worker can grab the queued resume job
 * before its JobDataStore context is written ("Missing job context" in the
 * server log) - the job is then dropped and the flow stays 'waiting'
 * forever. If the manager-approval step output hasn't been recorded ~10s
 * after task completion, re-issue the resume via the public resume endpoint
 * (it feeds the same resume path as inbox completion).
 */
async function approveTask(flows, inbox, instanceId, response) {
  const task = await findTask(inbox, instanceId);
  console.log(`  📥 inbox task: "${task.title}" [${task.task_type}, P${task.priority}]`);
  assert(task.task_type === 'approval', `expected an approval task, got ${task.task_type}`);
  assert(
    task.priority === 3,
    `expected priority 3 on the approval task, got ${task.priority}`,
  );

  const waiting = await flows.getInstanceStatus(instanceId);
  assert(
    waiting.status === 'waiting',
    `flow must pause for manager approval, got: ${waiting.status} ${waiting.error ?? ''}`,
  );
  console.log('✅ Flow is waiting for manager approval');

  const result = await inbox.completeTask(task.id, response);
  console.log(
    `✅ Task completed with "${response.action}", flow resuming (job`,
    result.flow?.job_id + ')',
  );

  // Evidence the resume actually happened: the human task's response gets
  // recorded as the manager-approval step output. ('waiting' alone is
  // ambiguous - later function steps also show as waiting.)
  for (let i = 0; i < 20; i++) {
    const status = await flows.getInstanceStatus(instanceId);
    if (
      stepOutputs(status)['manager-approval'] !== undefined ||
      status.status !== 'waiting'
    ) {
      return task;
    }
    await sleep(500);
  }
  console.log(
    '⚠️  flow still waiting after approval (known engine race: resume job ' +
      'dropped on "Missing job context") - re-issuing resume',
  );
  await flows.resume(instanceId, { ...response, completed_by: USERNAME });
  return task;
}

// ---------------------------------------------------------------------------
// Scenario A: engineer hire - laptop ordered, approval, welcome email
// ---------------------------------------------------------------------------

async function runEngineerHire(flows, inbox) {
  console.log('\n━━━ Scenario A: engineer hire (laptop + approval) ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    name: 'Ada Lovelace',
    role: 'engineer',
    start_date: '2026-07-01',
  });
  console.log('✅ Flow started:', instance_id);

  await approveTask(flows, inbox, instance_id, {
    action: 'approve',
    comment: 'Welcome to the team! (employee-onboarding example)',
  });

  const final = await waitForStatus(flows, instance_id, ['completed', 'failed']);
  assert(
    final.status === 'completed',
    `engineer hire must complete after approval, got: ${final.status} ${final.error ?? ''}`,
  );

  const outputs = stepOutputs(final);
  const accounts = outputs['create-accounts'] ?? {};
  const laptop = outputs['order-laptop'] ?? {};
  const welcome = outputs['send-welcome'] ?? {};
  const decision = final.variables?.__human_response ?? {};

  assert(
    accounts.email === 'ada.lovelace@example-corp.com',
    `expected provisioned email ada.lovelace@example-corp.com, got ${accounts.email}`,
  );
  assert(
    Array.isArray(accounts.systems) && accounts.systems.includes('github'),
    `engineers must get the github system account, got ${JSON.stringify(accounts.systems)}`,
  );
  console.log(`  👤 provisioned: ${accounts.email} (${accounts.account_id})`);

  assert(
    typeof laptop.order_id === 'string' && laptop.order_id.length > 0,
    `equipment gate must have routed an engineer to order-laptop, got ${JSON.stringify(laptop)}`,
  );
  assert(
    laptop.for_account === accounts.account_id,
    'laptop order must reference the account from create-accounts',
  );
  console.log(`  💻 laptop ordered: ${laptop.order_id} (${laptop.model})`);

  assert(
    decision.action === 'approve',
    `expected __human_response.action approve, got ${JSON.stringify(decision)}`,
  );
  console.log(`  ✔️  decision: ${decision.action} by ${decision.completed_by}`);

  assert(
    typeof welcome.welcome_text === 'string' &&
      welcome.welcome_text.includes(accounts.email),
    `welcome text must contain the provisioned email, got ${JSON.stringify(welcome)}`,
  );
  assert(welcome.sent === true, 'approved onboarding must mark the welcome email as sent');
  console.log(`  ✉️  "${welcome.welcome_text}"`);
  console.log('✅ Engineer onboarding completed');
}

// ---------------------------------------------------------------------------
// Scenario B: contractor hire - equipment gate skipped
// ---------------------------------------------------------------------------

async function runContractorHire(flows, inbox) {
  console.log('\n━━━ Scenario B: contractor hire (no laptop) ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    name: 'Grace Hopper',
    role: 'contractor',
    start_date: '2026-08-01',
  });
  console.log('✅ Flow started:', instance_id);

  await approveTask(flows, inbox, instance_id, {
    action: 'approve',
    comment: 'Contract approved (employee-onboarding example)',
  });

  const final = await waitForStatus(flows, instance_id, ['completed', 'failed']);
  assert(
    final.status === 'completed',
    `contractor hire must complete after approval, got: ${final.status} ${final.error ?? ''}`,
  );

  const outputs = stepOutputs(final);
  const accounts = outputs['create-accounts'] ?? {};
  const welcome = outputs['send-welcome'] ?? {};

  assert(
    outputs['order-laptop'] === undefined,
    `equipment gate must be SKIPPED for non-engineers, got ${JSON.stringify(
      outputs['order-laptop'],
    )}`,
  );
  console.log('✅ Equipment gate skipped (no rule matched for role=contractor)');

  assert(
    accounts.email === 'grace.hopper@example-corp.com',
    `expected provisioned email grace.hopper@example-corp.com, got ${accounts.email}`,
  );
  assert(
    Array.isArray(accounts.systems) && !accounts.systems.includes('github'),
    `contractors must NOT get engineer systems, got ${JSON.stringify(accounts.systems)}`,
  );
  assert(
    typeof welcome.welcome_text === 'string' &&
      welcome.welcome_text.includes(accounts.email) &&
      welcome.sent === true,
    `welcome email must be sent with the provisioned address, got ${JSON.stringify(welcome)}`,
  );
  console.log(`  ✉️  "${welcome.welcome_text}"`);
  console.log('✅ Contractor onboarding completed');
}

// ---------------------------------------------------------------------------
// Scenario C: manager rejects - flow completes with the rejection notice
// ---------------------------------------------------------------------------

async function runRejectedHire(flows, inbox) {
  console.log('\n━━━ Scenario C: manager rejects the onboarding ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    name: 'Charles Babbage',
    role: 'contractor',
    start_date: '2026-09-01',
  });
  console.log('✅ Flow started:', instance_id);

  await approveTask(flows, inbox, instance_id, {
    action: 'reject',
    comment: 'Position was filled internally (employee-onboarding example)',
  });

  // A rejection is data, not control flow: the flow still completes, and
  // send-welcome branches its text on __human_response.action.
  const final = await waitForStatus(flows, instance_id, ['completed', 'failed']);
  assert(
    final.status === 'completed',
    `rejected onboarding must still complete, got: ${final.status} ${final.error ?? ''}`,
  );

  const decision = final.variables?.__human_response ?? {};
  assert(
    decision.action === 'reject',
    `__human_response.action must be reject, got ${JSON.stringify(decision)}`,
  );
  console.log(`  ✔️  decision: ${decision.action} by ${decision.completed_by}`);

  const outputs = stepOutputs(final);
  const accounts = outputs['create-accounts'] ?? {};
  const welcome = outputs['send-welcome'] ?? {};
  assert(
    welcome.decision === 'reject',
    `send-welcome must SEE the rejection via \${__human_response.action}, got ${JSON.stringify(welcome)}`,
  );
  assert(
    typeof welcome.welcome_text === 'string' &&
      welcome.welcome_text.includes('was not approved') &&
      welcome.welcome_text.includes(accounts.email),
    `rejected hires must get the rejection-notice variant, got ${JSON.stringify(welcome)}`,
  );
  assert(welcome.sent === false, 'rejection notice must not count as a sent welcome email');
  console.log(`  ✉️  "${welcome.welcome_text}"`);
  console.log('✅ Rejection path verified (flow completed, rejected variant sent)');
}

// ---------------------------------------------------------------------------
// Scenario D: mail outage after approval - saga compensation deprovisions
// ---------------------------------------------------------------------------

async function runCompensatedHire(flows, inbox, client) {
  console.log('\n━━━ Scenario D: send-welcome outage -> saga rollback ━━━');

  const { instance_id } = await flows.run(FLOW_PATH, {
    name: 'Alan Turing',
    role: 'contractor',
    start_date: '2026-10-01',
    simulate_welcome_failure: true, // send-welcome throws after accounts were provisioned
  });
  console.log('✅ Flow started:', instance_id);

  // Compensation only pushes onto the saga stack when a step SUCCEEDS, so
  // the rollback demo needs a LATER step to fail: the manager approves,
  // then send-welcome (retry_strategy 'none') hits the simulated outage.
  await approveTask(flows, inbox, instance_id, {
    action: 'approve',
    comment: 'Approved - the mail gateway is about to ruin it (example)',
  });

  // The failing function is retried by the job system (~40s total) before
  // the flow sees the final failure and rolls back - poll up to 2 minutes.
  // PITFALL: the instance reads 'failed' TRANSIENTLY between the final step
  // failure and the end of compensation execution - only then does it flip
  // to 'rolled_back'. Treat 'failed' as "rollback in progress" and keep
  // polling for a grace period instead of asserting on first sight.
  console.log('  ⏳ waiting for the job retries to exhaust (~40s) ...');
  let final = await waitForStatus(
    flows,
    instance_id,
    ['rolled_back', 'failed', 'completed'],
    240,
  );
  if (final.status === 'failed') {
    console.log('  ⏳ step failed - waiting for the saga compensation to finish ...');
    final = await waitForStatus(flows, instance_id, ['rolled_back'], 120);
  }
  assert(
    final.status === 'rolled_back',
    `expected saga rollback, got: ${final.status} ${final.error ?? ''} ` +
      `(variables: ${JSON.stringify(final.variables)})`,
  );
  console.log(`✅ Flow rolled back (error: ${final.error ?? 'simulated outage'})`);

  const outputs = stepOutputs(final);
  const accounts = outputs['create-accounts'] ?? {};
  assert(
    typeof accounts.account_id === 'string',
    'create-accounts must have succeeded before the rollback',
  );
  assert(
    outputs['send-welcome'] === undefined,
    'send-welcome must NOT have produced output',
  );

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
  const entry = stack.find((e) => e.step_id === 'create-accounts');
  assert(entry, `no compensation entry for 'create-accounts' in ${JSON.stringify(stack)}`);
  assert(
    entry.compensation_status?.status === 'executed',
    `compensation must be executed, got ${JSON.stringify(entry.compensation_status)}`,
  );
  assert(
    entry.compensation_input?.account_id === accounts.account_id,
    'compensation_input_mapping must pass ${output.account_id} to deprovision-accounts',
  );
  console.log(
    `  ↩️  compensation executed: ${entry.compensation_fn} ` +
      `({ account_id: ${entry.compensation_input.account_id} })`,
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

  await runEngineerHire(flows, inbox);
  await runContractorHire(flows, inbox);
  await runRejectedHire(flows, inbox);
  await runCompensatedHire(flows, inbox, client);

  console.log('\n🎉 All scenarios passed.');
}

main().catch((err) => {
  console.error('❌', err.message ?? err);
  process.exit(1);
});
