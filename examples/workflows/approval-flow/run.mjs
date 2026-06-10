#!/usr/bin/env node
/**
 * End-to-end human-in-the-loop workflow example.
 *
 * Demonstrates the full loop with the @raisindb/client SDK:
 *   1. Define a flow: function-ish steps + a human approval task
 *   2. Run it and stream execution events (SSE)
 *   3. When the flow pauses on the approval, find the task in the inbox
 *   4. Complete the task -> the flow resumes and finishes
 *
 * Prereqs: a running raisin-server (dev mode) on RAISIN_URL.
 *   RUST_LOG=info ./target/release/raisin-server --config ... --dev-mode
 *
 * Run:
 *   npm install && npm start
 */

import { RaisinHttpClient, FlowClient, InboxApi } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'workflow-demo';
const USERNAME = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const FLOW_PATH = '/flows/order-approval';

// The workflow definition in DESIGNER format - the same format the admin
// console's visual flow designer reads and writes, converted to the runtime
// format by the engine when the flow runs. Templates like
// {{ input.order_id }} resolve against the flow input.
const workflowData = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    {
      id: 'approve',
      node_type: 'raisin:FlowStep',
      properties: {
        action: 'Approve order {{ input.order_id }} ({{ input.amount }} CHF)',
        step_type: 'human_task',
        task_type: 'approval',
        assignee: '/users/admin',
        task_description: 'A new order needs your approval before fulfillment.',
        priority: 4,
        options: [
          { value: 'approve', label: 'Approve', style: 'success' },
          { value: 'reject', label: 'Reject', style: 'danger' },
        ],
      },
    },
  ],
};

async function api(client, method, path, body) {
  const token = client.getAuthManager().getAccessToken();
  const response = await fetch(`${BASE_URL}${path}`, {
    method,
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  return response;
}

async function ensureSetup(client) {
  // Repository (idempotent - ignore "already exists")
  await api(client, 'POST', '/api/repositories', { repo_id: REPO });

  // /flows folder + the flow node in the functions workspace (retried
  // below because node types initialize asynchronously after repo creation)
  for (let i = 0; i < 20; i++) {
    const folder = await api(client, 'POST', `/api/repository/${REPO}/main/head/functions/`, {
      node: { name: 'flows', node_type: 'raisin:Folder', properties: {} },
    });
    if (folder.ok || /exists|conflict/i.test(await folder.text())) break;
    await new Promise((r) => setTimeout(r, 1000));
  }
  // Node types initialize asynchronously right after repo creation - retry
  let lastError = '';
  for (let i = 0; i < 20; i++) {
    const created = await api(
      client,
      'POST',
      `/api/repository/${REPO}/main/head/functions/flows`,
      {
        node: {
          name: 'order-approval',
          node_type: 'raisin:Flow',
          properties: {
            name: 'order-approval',
            title: 'Order Approval',
            enabled: true,
            workflow_data: workflowData,
          },
        },
      },
    );
    if (created.ok) {
      console.log('✅ Flow deployed at', FLOW_PATH);
      return;
    }
    lastError = await created.text();
    if (/exists|conflict/i.test(lastError)) {
      // Refresh the definition so edits to workflowData take effect
      const updated = await api(
        client,
        'PUT',
        `/api/repository/${REPO}/main/head/functions/flows/order-approval`,
        {
          properties: {
            name: 'order-approval',
            title: 'Order Approval',
            enabled: true,
            workflow_data: workflowData,
          },
        },
      );
      console.log(
        updated.ok
          ? 'ℹ️  Flow node already existed - definition refreshed'
          : 'ℹ️  Flow node already exists, reusing (refresh failed)',
      );
      return;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`Failed to deploy flow: ${lastError}`);
}

async function main() {
  // 1. Connect + authenticate
  const client = new RaisinHttpClient(BASE_URL, { tenantId: 'default' });
  await client.authenticate({ username: USERNAME, password: PASSWORD });
  console.log('✅ Authenticated');

  await ensureSetup(client);

  const flows = FlowClient.fromHttpClient(client, BASE_URL, REPO);
  const inbox = new InboxApi(BASE_URL, REPO, client.getAuthManager());

  // 2. Start the flow - it runs until the approval and pauses
  const { instance_id } = await flows.run(FLOW_PATH, {
    order_id: 'ORD-1042',
    amount: 249,
  });
  console.log('✅ Flow started:', instance_id);

  // 3. Wait until the flow pauses on the human task
  const status = await waitForStatus(flows, instance_id, [
    'waiting',
    'completed',
    'failed',
  ]);
  if (status.status !== 'waiting') {
    throw new Error(`Expected the flow to wait for approval, got: ${status.status}`);
  }
  console.log('  ⏸ flow is waiting for approval');

  // 4. Find the approval task in the inbox
  const task = await findTask(inbox, instance_id);
  console.log(`  📥 inbox task: "${task.title}" [${task.task_type}, P${task.priority}]`);

  // 5. Subscribe to events BEFORE resuming so nothing is missed
  const stream = await flows.createEventStream(instance_id);

  // 6. Complete the task -> the flow resumes
  const result = await inbox.completeTask(task.id, {
    action: 'approve',
    comment: 'Approved from the SDK example',
  });
  console.log('  ✅ task completed, flow resuming (job', result.flow?.job_id + ')');

  // 7. Stream the rest of the execution
  for await (const event of stream.events) {
    switch (event.type) {
      case 'step_started':
        console.log(`  ▶ step ${event.node_id}`);
        break;
      case 'step_completed':
        console.log(`  ✔ step ${event.node_id} (${event.duration_ms}ms)`);
        break;
      case 'flow_completed': {
        console.log('🎉 Flow completed.');
        const final = await flows.getInstanceStatus(instance_id);
        console.log(
          '  Decision:',
          final.variables?.__human_response?.action,
          'by',
          final.variables?.__human_response?.completed_by,
        );
        stream.close();
        return;
      }
      case 'flow_failed':
        stream.close();
        throw new Error(`Flow failed: ${event.error}`);
      default:
        break;
    }
  }
}

async function waitForStatus(flows, instanceId, wanted, attempts = 60) {
  for (let i = 0; i < attempts; i++) {
    try {
      const status = await flows.getInstanceStatus(instanceId);
      if (wanted.includes(status.status)) return status;
    } catch {
      // instance node may not be visible yet - keep polling
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error('Timed out waiting for the flow to reach ' + wanted.join('/'));
}

async function findTask(inbox, instanceId, attempts = 10) {
  for (let i = 0; i < attempts; i++) {
    const { tasks } = await inbox.listTasks({
      status: 'pending',
      assignee: '/users/admin',
    });
    const task = tasks.find((t) => t.flow_instance_id === instanceId);
    if (task) return task;
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error('Approval task did not appear in the inbox');
}

main().catch((err) => {
  console.error('❌', err.message ?? err);
  process.exit(1);
});
