#!/usr/bin/env node
/**
 * AI workflow example: agent step + agent-in-the-loop.
 *
 * Functions, AI agents, and humans are uniform step concepts in RaisinDB
 * workflows. This example shows both AI roles:
 *
 *   1. An `agent_step` summarizes the request (one-shot AI call with a
 *      templated prompt).
 *   2. A `human_task` whose ASSIGNEE is an AI agent: the agent receives the
 *      approval task, answers with a structured decision
 *      `{ decision, reasoning, confidence }`, and a confident decision
 *      completes the task automatically. On errors or low confidence the
 *      task ESCALATES to the human `escalation_assignee` - and this script
 *      then completes it as the human would (same API either way).
 *
 * Prereqs: a running raisin-server with an AI provider configured for the
 * agent (model/provider properties below; adjust to your setup). Without a
 * working provider the example still completes - via the escalation path.
 */

import { RaisinHttpClient, FlowClient, InboxApi } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'workflow-demo';
const USERNAME = process.env.RAISIN_USER ?? 'admin';
const PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const AGENT_MODEL = process.env.RAISIN_AGENT_MODEL ?? 'gpt-4o-mini';
const AGENT_PROVIDER = process.env.RAISIN_AGENT_PROVIDER ?? 'openai';

const FLOW_PATH = '/flows/refund-approval';
const AGENT_PATH = '/agents/refund-approver';

const workflowData = {
  version: 1,
  error_strategy: 'fail_fast',
  nodes: [
    { id: 'start', step_type: 'start', next_node: 'summarize' },
    {
      // One-shot AI call: prompt is template-resolved against the input
      id: 'summarize',
      step_type: 'agent_step',
      properties: {
        agent_ref: AGENT_PATH,
        prompt:
          'Summarize this refund request in one sentence: ' +
          'customer {{ input.customer }}, amount {{ input.amount }} CHF, ' +
          'reason: {{ input.reason }}',
        next_node: 'approve',
      },
    },
    {
      // Human task assigned to an AI AGENT. A confident structured decision
      // completes it; otherwise it escalates to the human below.
      id: 'approve',
      step_type: 'human_task',
      properties: {
        task_type: 'approval',
        title: 'Refund {{ input.amount }} CHF for {{ input.customer }}?',
        description: 'Summary: {{ steps.summarize.response }}',
        assignee: AGENT_PATH,
        min_confidence: 0.75,
        escalation_assignee: '/users/admin',
        options: [
          { value: 'approve', label: 'Approve refund', style: 'success' },
          { value: 'reject', label: 'Reject', style: 'danger' },
        ],
      },
      next_node: 'end',
    },
    { id: 'end', step_type: 'end' },
  ],
};

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

async function ensureSetup(client) {
  await api(client, 'POST', '/api/repositories', { repo_id: REPO });

  for (const folder of ['flows', 'agents']) {
    await api(client, 'POST', `/api/repository/${REPO}/main/head/functions/`, {
      node: { name: folder, node_type: 'raisin:Folder', properties: {} },
    });
  }

  // The AI agent (also the task assignee)
  await api(client, 'POST', `/api/repository/${REPO}/main/head/functions/agents`, {
    node: {
      name: 'refund-approver',
      node_type: 'raisin:AIAgent',
      properties: {
        name: 'refund-approver',
        title: 'Refund Approver',
        system_prompt:
          'You review refund requests. Approve refunds under 500 CHF with a ' +
          'plausible reason. Be conservative: if unsure, say so with low confidence.',
        provider: AGENT_PROVIDER,
        model: AGENT_MODEL,
        temperature: 0.2,
        enabled: true,
        tools: [],
      },
    },
  });

  await api(client, 'POST', `/api/repository/${REPO}/main/head/functions/flows`, {
    node: {
      name: 'refund-approval',
      node_type: 'raisin:Flow',
      properties: {
        name: 'refund-approval',
        title: 'Refund Approval (AI in the loop)',
        enabled: true,
        workflow_data: workflowData,
      },
    },
  });
  console.log('✅ Agent + flow deployed');
}

async function main() {
  const client = new RaisinHttpClient(BASE_URL, { tenantId: 'default' });
  await client.authenticate({ username: USERNAME, password: PASSWORD });
  console.log('✅ Authenticated');

  await ensureSetup(client);

  const flows = FlowClient.fromHttpClient(client, BASE_URL, REPO);
  const inbox = new InboxApi(BASE_URL, REPO, client.getAuthManager());

  const { instance_id } = await flows.run(FLOW_PATH, {
    customer: 'ACME GmbH',
    amount: 120,
    reason: 'damaged packaging on arrival',
  });
  console.log('✅ Flow started:', instance_id);

  const stream = await flows.createEventStream(instance_id);

  for await (const event of stream.events) {
    switch (event.type) {
      case 'step_started':
        console.log(`  ▶ ${event.node_id}`);
        break;
      case 'text_chunk':
        process.stdout.write(event.text);
        break;
      case 'step_completed':
        console.log(`\n  ✔ ${event.node_id}`);
        break;
      case 'flow_waiting': {
        // The agent could not decide (no provider / low confidence) and the
        // task escalated to the human assignee - complete it as the human.
        console.log('  ⏸ escalated to human, completing via inbox API...');
        const { tasks } = await inbox.listTasks({ status: 'pending' });
        const task = tasks.find((t) => t.flow_instance_id === instance_id);
        if (task) {
          if (task.escalated_from) {
            console.log(
              `  📥 task escalated from ${task.escalated_from}: ${task.escalation_reason}`,
            );
          }
          await inbox.completeTask(task.id, {
            action: 'approve',
            comment: 'Approved by human after escalation',
          });
          console.log('  ✅ completed as human');
        }
        break;
      }
      case 'flow_completed': {
        console.log('🎉 Flow completed.');
        const status = await flows.getInstanceStatus(instance_id);
        const response = status.variables?.__human_response;
        if (response) {
          console.log(
            `  Decision: ${response.action} (by ${response.completed_by ?? 'agent'})`,
          );
        }
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

main().catch((err) => {
  console.error('❌', err.message ?? err);
  process.exit(1);
});
