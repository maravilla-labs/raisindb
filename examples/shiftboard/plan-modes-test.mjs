#!/usr/bin/env node
/**
 * Shiftboard demo - PLAN/TASK system e2e test (all four execution modes +
 * task_creation_enabled gate + JS-SDK plan approval).
 *
 * Creates a DEDICATED agent /agents/plan-tester (the shift-planner demo agent
 * is not touched) wired to the four planning tools + list-shifts, then drives
 * one conversation per execution mode through the JS SDK as the planner user:
 *
 *   1. manual            - plan created + pending_approval, NO execution
 *   2. approve_then_auto - approvePlan() -> all tasks run to completion;
 *                          second plan rejectPlan() -> cancelled, no execution
 *   3. step_by_step      - approve -> exactly one task per continue signal
 *   4. automatic         - tasks created AND executed without approval
 *   5. task_creation_enabled=false - planning tools filtered, no plan nodes
 *
 * SDK-side state (ConversationStore.plans projection, approvePlan/rejectPlan)
 * is asserted at each stage; node-side state (raisin:AIPlan / raisin:AITask in
 * the `ai` workspace) is asserted with an admin session.
 *
 * Costs real Groq tokens (~15-25 LLM calls) - run sparingly.
 *
 * Run: npm run plan-modes-test                       (all scenarios)
 *      node plan-modes-test.mjs --mode manual         (single scenario)
 *      node plan-modes-test.mjs --keep                (skip cleanup)
 */

import { RaisinClient, MemoryTokenStorage, ConversationStore } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard2';
const WS_URL =
  process.env.RAISIN_WS_URL ??
  (TENANT === 'default'
    ? `${BASE_URL.replace(/^http/, 'ws')}/ws/${REPO}`
    : `${BASE_URL.replace(/^http/, 'ws')}/sys/${TENANT}/${REPO}`);
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const PLANNER = { email: 'planner@example.com', password: 'Planner12345!' };

const AGENT_NAME = 'plan-tester';
const AGENT_PATH = `/agents/${AGENT_NAME}`;
const AGENT_MODEL = process.env.AGENT_MODEL ?? 'llama-3.3-70b-versatile';

const ARGS = process.argv.slice(2);
const ONLY_MODE = (() => {
  const i = ARGS.indexOf('--mode');
  return i >= 0 ? ARGS[i + 1] : null;
})();
const KEEP = ARGS.includes('--keep');

const PLAN_PROMPT =
  'Plan: step 1 list the shifts, step 2 summarize them. Create the plan now.';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function say(tag, text) {
  console.log(`\n[${tag}] ${text}`);
}

let failures = 0;
function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
  console.log(`   ✓ ${message}`);
}

async function pollFor(label, fn, timeoutMs = 90_000, intervalMs = 2_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await fn();
    if (value) return value;
    if (Date.now() > deadline) {
      throw new Error(`Timed out after ${timeoutMs}ms waiting for: ${label}`);
    }
    await sleep(intervalMs);
  }
}

// ───────────────────────── admin REST helpers ─────────────────────────

let adminToken = null;

async function adminApi(method, path, body) {
  const res = await fetch(`${BASE_URL}${path}`, {
    method,
    headers: {
      'Content-Type': 'application/json',
      ...(adminToken ? { Authorization: `Bearer ${adminToken}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return res;
}

async function adminLogin() {
  const res = await adminApi('POST', `/api/raisindb/sys/${TENANT}/auth`, {
    username: ADMIN_USER,
    password: ADMIN_PASSWORD,
  });
  if (!res.ok) throw new Error(`admin login failed: HTTP ${res.status}`);
  const data = await res.json();
  adminToken = data.token;
}

async function adminSql(sql, params = []) {
  const res = await adminApi('POST', `/api/sql/${REPO}`, { sql, params });
  if (!res.ok) throw new Error(`admin sql failed: HTTP ${res.status} ${await res.text()}`);
  const data = await res.json();
  return data.rows ?? [];
}

async function ensureNode(workspace, parentPath, name, nodeType, properties) {
  const base = `/api/repository/${REPO}/main/head/${workspace}`;
  const create = await adminApi('POST', `${base}${parentPath === '/' ? '' : parentPath}/`, {
    name,
    node_type: nodeType,
    properties,
  });
  if (create.ok) return 'created';
  const text = await create.text();
  if (/exists/i.test(text)) {
    const childPath = parentPath === '/' ? `/${name}` : `${parentPath.replace(/\/$/, '')}/${name}`;
    const updated = await adminApi('PUT', `${base}${childPath}`, { properties });
    if (!updated.ok) throw new Error(`update ${workspace}:${childPath} failed: ${await updated.text()}`);
    return 'updated';
  }
  throw new Error(`create ${workspace}:${parentPath}/${name} failed: ${text}`);
}

async function deleteNode(workspace, path) {
  // NOTE: the HTTP DELETE endpoint currently returns {"deleted":true} without
  // actually removing the node (tombstone not visible to reads — regression
  // under investigation), so cleanup uses SQL DELETE which works.
  await adminSql(`DELETE FROM '${workspace}' WHERE DESCENDANT_OF($1)`, [path]).catch(() => {});
  const rows = await adminSql(`DELETE FROM '${workspace}' WHERE path = $1`, [path]).catch(() => []);
  return Array.isArray(rows);
}

// ───────────────────────── agent deployment ─────────────────────────

function agentSystemPrompt() {
  return [
    'You are a planning test assistant for a shift board.',
    '',
    "When a user message starts with 'Plan:', you MUST call the create-plan",
    'tool with exactly TWO tasks:',
    '  task 1: title "List shifts" - when executing it, call the list-shifts tool.',
    '  task 2: title "Summarize shifts" - when executing it, write a one-sentence',
    '          text summary of the shifts; no tool needed.',
    '',
    'When executing plan tasks, always follow the Task Planning instructions:',
    'mark each task in_progress with update-task before working on it and',
    'completed when done (use the task_id values returned by create-plan).',
    '',
    "If the message does not start with 'Plan:', answer directly in one short",
    'sentence without creating a plan.',
    'Keep every text response under two sentences.',
  ].join('\n');
}

function agentProperties(overrides = {}) {
  return {
    title: 'Plan Tester',
    system_prompt: agentSystemPrompt(),
    provider: 'groq',
    model: AGENT_MODEL,
    temperature: 0.1,
    max_tokens: 1024,
    thinking_enabled: false,
    execution_context: 'system',
    task_creation_enabled: true,
    execution_mode: 'manual',
    tools: [
      '/lib/raisin/ai/create-plan',
      '/lib/raisin/ai/add-task',
      '/lib/raisin/ai/update-task',
      '/lib/raisin/ai/get-plan-status',
      '/lib/shiftboard/list-shifts',
    ],
    ...overrides,
  };
}

async function deployAgent() {
  await ensureNode('functions', '/', 'agents', 'raisin:Folder', {});
  await ensureNode('functions', '/agents', AGENT_NAME, 'raisin:AIAgent', agentProperties());

  // Agent home in the `ai` workspace (mirrors setup.mjs shift-planner home)
  await ensureNode('ai', '/agents', AGENT_NAME, 'raisin:Folder', {
    title: 'Plan Tester',
    agent_ref: {
      'raisin:ref': '',
      'raisin:workspace': 'functions',
      'raisin:path': AGENT_PATH,
    },
    user_id: `agent:${AGENT_NAME}`,
    display_name: 'Plan Tester',
    max_turns: 10,
  });
  for (const [parent, name, title] of [
    [AGENT_PATH, 'inbox', 'Inbox'],
    [AGENT_PATH, 'outbox', 'Outbox'],
    [AGENT_PATH, 'memory', 'User Memory'],
    [AGENT_PATH, 'sent', 'Sent'],
    [`${AGENT_PATH}/inbox`, 'chats', 'Chats'],
    [`${AGENT_PATH}/inbox`, 'notifications', 'Notifications'],
  ]) {
    await ensureNode('ai', parent, name, 'raisin:Folder', { title });
  }
  say('setup', `Agent ${AGENT_PATH} deployed (groq ${AGENT_MODEL})`);
}

async function setAgentConfig(overrides) {
  const res = await adminApi('PUT', `/api/repository/${REPO}/main/head/functions${AGENT_PATH}`, {
    properties: agentProperties(overrides),
  });
  if (!res.ok) throw new Error(`agent config update failed: ${await res.text()}`);
  say('setup', `Agent config: ${JSON.stringify(overrides)}`);
}

// ───────────────────────── node-side assertions ─────────────────────────

function agentChatPath(conversationPath) {
  const id = conversationPath.split('/').pop();
  return `${AGENT_PATH}/inbox/chats/${id}`;
}

async function getPlanNodes(chatPath) {
  return adminSql(
    `SELECT id, path, properties FROM 'ai'
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIPlan'
     ORDER BY created_at ASC`,
    [chatPath],
  );
}

async function getTaskNodes(planPath) {
  return adminSql(
    `SELECT id, path, properties FROM 'ai'
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'
     ORDER BY created_at ASC`,
    [planPath],
  );
}

async function getToolCallNames(chatPath) {
  const rows = await adminSql(
    `SELECT path, properties->>'function_name'::String AS fn FROM 'ai'
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIToolCall'
     ORDER BY created_at ASC`,
    [chatPath],
  );
  return rows.map((r) => r.fn);
}

async function costSummary() {
  const rows = await adminSql(
    `SELECT properties FROM 'ai'
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
    [AGENT_PATH],
  );
  let calls = 0;
  let tokens = 0;
  for (const r of rows) {
    calls += 1;
    tokens += Number(r.properties?.total_tokens ?? 0) || 0;
  }
  return { calls, tokens };
}

// ───────────────────────── SDK conversation harness ─────────────────────────

function newStore(db, conversationPath) {
  return new ConversationStore({
    database: db,
    conversationPath,
    streamingTimeoutMs: 120_000,
  });
}

/** Send a message and wait for the turn to reach a resting state. */
async function sendAndSettle(store, text, label) {
  say('user', text);
  await store.sendMessage(text);
  await pollFor(`${label}: turn settled`, () => {
    const s = store.getSnapshot();
    return !s.isStreaming && s.activeToolCalls.length === 0 ? s : null;
  }, 120_000, 1_500);
}

/** Poll the SDK plan projection (reloading messages each try). */
async function pollSdkPlan(store, predicate, label, timeoutMs = 90_000) {
  return pollFor(label, async () => {
    await store.loadMessages();
    const plans = store.getSnapshot().plans;
    return predicate(plans) || null;
  }, timeoutMs, 2_500);
}

// ───────────────────────── scenarios ─────────────────────────

const createdConversations = [];

async function newConversation(db, subject) {
  const convo = await db.conversations.create({
    participant: AGENT_PATH,
    subject,
  });
  createdConversations.push(convo.conversationPath);
  say('setup', `Conversation: ${convo.conversationPath}`);
  return convo;
}

async function scenarioManual(db) {
  say('SCENARIO', 'manual - plan is created but nothing executes');
  await setAgentConfig({ execution_mode: 'manual', task_creation_enabled: true });
  const convo = await newConversation(db, 'plan-modes manual');
  const chatPath = agentChatPath(convo.conversationPath);
  const store = newStore(db, convo.conversationPath);

  try {
    await sendAndSettle(store, PLAN_PROMPT, 'manual');

    // Node-side: plan + 2 tasks exist, plan pending_approval
    const plan = (await pollFor('manual: AIPlan node', async () => {
      const plans = await getPlanNodes(chatPath);
      return plans.length > 0 ? plans[0] : null;
    }));
    assert(plan.properties?.status === 'pending_approval',
      `plan status is pending_approval (got ${plan.properties?.status})`);
    const tasks = await pollFor('manual: 2 AITask nodes', async () => {
      const t = await getTaskNodes(plan.path);
      return t.length === 2 ? t : null;
    });
    assert(tasks.every((t) => (t.properties?.status ?? 'pending') === 'pending'),
      'both tasks are pending');

    // SDK-side: plan projection visible with pending approval
    const sdkPlans = await pollSdkPlan(store,
      (plans) => (plans.length > 0 && plans[0].requiresApproval ? plans : null),
      'manual: SDK plan projection');
    assert(sdkPlans[0].planPath === plan.path,
      `SDK plan path matches node path (${sdkPlans[0].planPath})`);
    assert(sdkPlans[0].tasks.length === 2, 'SDK projection has 2 tasks');
    assert(sdkPlans[0].status === 'pending_approval', 'SDK plan status pending_approval');

    // No execution: wait, then verify no update-task / list-shifts tool calls
    await sleep(10_000);
    const toolCalls = await getToolCallNames(chatPath);
    const executionCalls = toolCalls.filter((n) =>
      ['update-task', 'update_task', 'list-shifts', 'list_shifts'].includes(n));
    assert(executionCalls.length === 0,
      `no execution tool calls in manual mode (got ${JSON.stringify(toolCalls)})`);
    const tasksAfter = await getTaskNodes(plan.path);
    assert(tasksAfter.every((t) => (t.properties?.status ?? 'pending') === 'pending'),
      'tasks still pending after waiting');
  } finally {
    store.destroy();
  }
}

async function scenarioApproveThenAuto(db) {
  say('SCENARIO', 'approve_then_auto - approval gates execution, then runs all');
  await setAgentConfig({ execution_mode: 'approve_then_auto', task_creation_enabled: true });
  const convo = await newConversation(db, 'plan-modes approve_then_auto');
  const chatPath = agentChatPath(convo.conversationPath);
  const store = newStore(db, convo.conversationPath);

  try {
    await sendAndSettle(store, PLAN_PROMPT, 'approve_then_auto');

    const plan = await pollFor('approve_then_auto: AIPlan node', async () => {
      const plans = await getPlanNodes(chatPath);
      return plans.length > 0 ? plans[0] : null;
    });
    assert(plan.properties?.status === 'pending_approval', 'plan awaits approval');

    const sdkPlans = await pollSdkPlan(store,
      (plans) => (plans.some((p) => p.planPath === plan.path) ? plans : null),
      'approve_then_auto: SDK plan projection');
    const sdkPlan = sdkPlans.find((p) => p.planPath === plan.path);
    assert(sdkPlan.requiresApproval === true, 'SDK plan requires approval');

    // Approve via the SDK
    say('user', `approvePlan(${plan.path})`);
    await store.approvePlan(plan.path);

    // Tasks execute automatically to completion
    await pollFor('approve_then_auto: all tasks completed', async () => {
      const tasks = await getTaskNodes(plan.path);
      return tasks.length === 2 && tasks.every((t) => t.properties?.status === 'completed')
        ? tasks : null;
    }, 180_000, 3_000);
    assert(true, 'both tasks completed automatically after approval');

    const planAfter = (await getPlanNodes(chatPath)).find((p) => p.path === plan.path);
    assert(planAfter.properties?.status === 'completed',
      `plan node marked completed (got ${planAfter.properties?.status})`);

    const toolCalls = await getToolCallNames(chatPath);
    assert(toolCalls.some((n) => n === 'list-shifts' || n === 'list_shifts'),
      'list-shifts was actually executed as part of the plan');

    // SDK projection reflects completion
    await pollSdkPlan(store,
      (plans) => {
        const p = plans.find((x) => x.planPath === plan.path);
        return p && p.status === 'completed' && p.requiresApproval === false ? plans : null;
      },
      'approve_then_auto: SDK projection shows completed');
    assert(true, 'SDK projection shows plan completed');

    // ── Rejection: second plan in the same conversation ──
    await pollFor('approve_then_auto: turn fully settled', async () => {
      const s = store.getSnapshot();
      return !s.isStreaming ? s : null;
    }, 60_000, 2_000);
    await sendAndSettle(store, PLAN_PROMPT, 'approve_then_auto reject');

    const plan2 = await pollFor('approve_then_auto: second AIPlan node', async () => {
      const plans = await getPlanNodes(chatPath);
      const fresh = plans.filter((p) => p.path !== plan.path && p.properties?.status === 'pending_approval');
      return fresh.length > 0 ? fresh[fresh.length - 1] : null;
    });

    say('user', `rejectPlan(${plan2.path})`);
    await store.rejectPlan(plan2.path, 'Not needed anymore, stop here.');

    await pollFor('approve_then_auto: rejected plan cancelled', async () => {
      const plans = await getPlanNodes(chatPath);
      const p = plans.find((x) => x.path === plan2.path);
      return p?.properties?.status === 'cancelled' ? p : null;
    }, 60_000, 2_000);
    assert(true, 'rejected plan marked cancelled');

    // Give the rejection continuation a moment, then verify the rejected
    // plan's tasks were never executed.
    await sleep(15_000);
    const tasks2 = await getTaskNodes(plan2.path);
    assert(tasks2.every((t) => ['pending', 'cancelled'].includes(t.properties?.status ?? 'pending')),
      'rejected plan tasks were not executed');
  } finally {
    store.destroy();
  }
}

async function scenarioStepByStep(db) {
  say('SCENARIO', 'step_by_step - one task per continue signal');
  await setAgentConfig({ execution_mode: 'step_by_step', task_creation_enabled: true });
  const convo = await newConversation(db, 'plan-modes step_by_step');
  const chatPath = agentChatPath(convo.conversationPath);
  const store = newStore(db, convo.conversationPath);

  try {
    await sendAndSettle(store, PLAN_PROMPT, 'step_by_step');

    const plan = await pollFor('step_by_step: AIPlan node', async () => {
      const plans = await getPlanNodes(chatPath);
      return plans.length > 0 ? plans[0] : null;
    });
    assert(plan.properties?.status === 'pending_approval', 'plan awaits approval');

    say('user', `approvePlan(${plan.path})`);
    await store.approvePlan(plan.path);

    // Exactly ONE task completes, then the agent pauses
    await pollFor('step_by_step: first task completed', async () => {
      const tasks = await getTaskNodes(plan.path);
      return tasks.some((t) => t.properties?.status === 'completed') ? tasks : null;
    }, 180_000, 3_000);

    // Wait for the pause (awaiting_step_continue terminal state)
    await pollFor('step_by_step: agent paused after one task', async () => {
      const rows = await adminSql(
        `SELECT properties->>'finish_reason'::String AS fr,
                properties->>'dispatch_phase'::String AS phase
         FROM 'ai'
         WHERE CHILD_OF($1) AND node_type = 'raisin:Message'
           AND properties->>'role'::String = 'assistant'
         ORDER BY created_at DESC LIMIT 1`,
        [chatPath],
      );
      return rows[0]?.fr === 'awaiting_step_continue' ? rows[0] : null;
    }, 120_000, 3_000);
    assert(true, 'agent paused with awaiting_step_continue');

    let tasks = await getTaskNodes(plan.path);
    const completedCount = tasks.filter((t) => t.properties?.status === 'completed').length;
    assert(completedCount === 1,
      `exactly one task completed after approval (got ${completedCount})`);

    // Continue signal -> second task executes
    await sendAndSettle(store, 'continue', 'step_by_step continue');
    await pollFor('step_by_step: second task completed', async () => {
      const t = await getTaskNodes(plan.path);
      return t.length === 2 && t.every((x) => x.properties?.status === 'completed') ? t : null;
    }, 180_000, 3_000);
    assert(true, 'second task completed after continue signal');

    const planAfter = (await getPlanNodes(chatPath)).find((p) => p.path === plan.path);
    assert(planAfter.properties?.status === 'completed', 'plan completed after both steps');
  } finally {
    store.destroy();
  }
}

async function scenarioAutomatic(db) {
  say('SCENARIO', 'automatic - plan executes immediately without approval');
  await setAgentConfig({ execution_mode: 'automatic', task_creation_enabled: true });
  const convo = await newConversation(db, 'plan-modes automatic');
  const chatPath = agentChatPath(convo.conversationPath);
  const store = newStore(db, convo.conversationPath);

  try {
    await sendAndSettle(store, PLAN_PROMPT, 'automatic');

    const plan = await pollFor('automatic: AIPlan node', async () => {
      const plans = await getPlanNodes(chatPath);
      return plans.length > 0 ? plans[0] : null;
    });
    assert(plan.properties?.status !== 'pending_approval',
      `plan does not wait for approval (status ${plan.properties?.status})`);

    await pollFor('automatic: all tasks completed', async () => {
      const tasks = await getTaskNodes(plan.path);
      return tasks.length === 2 && tasks.every((t) => t.properties?.status === 'completed')
        ? tasks : null;
    }, 180_000, 3_000);
    assert(true, 'both tasks completed without any approval');

    const planAfter = (await getPlanNodes(chatPath)).find((p) => p.path === plan.path);
    assert(planAfter.properties?.status === 'completed', 'plan node completed');

    // SDK projection picked it up too
    await pollSdkPlan(store,
      (plans) => {
        const p = plans.find((x) => x.planPath === plan.path);
        return p && p.status === 'completed' ? plans : null;
      },
      'automatic: SDK projection shows completed');
    assert(true, 'SDK projection shows completed plan');
  } finally {
    store.destroy();
  }
}

async function scenarioCreationDisabled(db) {
  say('SCENARIO', 'task_creation_enabled=false - planning tools filtered out');
  await setAgentConfig({ execution_mode: 'automatic', task_creation_enabled: false });
  const convo = await newConversation(db, 'plan-modes disabled');
  const chatPath = agentChatPath(convo.conversationPath);
  const store = newStore(db, convo.conversationPath);

  try {
    await sendAndSettle(store, PLAN_PROMPT, 'disabled');

    // Wait a bit for any (incorrect) async plan creation to surface
    await sleep(8_000);
    const plans = await getPlanNodes(chatPath);
    assert(plans.length === 0, 'no AIPlan nodes created');

    // Planning tools are filtered from the model's tool list. The model may
    // still TRY to call them (they're named in the prompt); those attempts
    // must be recorded as unknown-tool error placeholders (no function_ref,
    // never executed) — which is exactly what "not available" means here.
    const planningCallRows = await adminSql(
      `SELECT path, properties FROM 'ai'
       WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIToolCall'
         AND properties->>'function_name'::String = 'create-plan'`,
      [chatPath],
    );
    for (const row of planningCallRows) {
      assert(!row.properties?.function_ref,
        `planning call ${row.path} is an unknown-tool placeholder (no function_ref)`);
      assert(row.properties?.status === 'completed',
        'placeholder call not dispatched for execution');
    }
    if (planningCallRows.length === 0) {
      assert(true, 'model did not attempt any planning tool call');
    }

    await store.loadMessages();
    const snapshot = store.getSnapshot();
    assert(snapshot.plans.length === 0, 'SDK plan projection is empty');
    const lastAssistant = [...snapshot.messages].reverse().find((m) => m.role === 'assistant');
    assert(lastAssistant && (lastAssistant.content ?? '').trim().length > 0,
      'agent answered directly with text');
    say('agent', lastAssistant.content.slice(0, 200));
  } finally {
    store.destroy();
  }
}

// ───────────────────────── cleanup ─────────────────────────

async function cleanup(db) {
  say('cleanup', 'Removing test agent conversations, plan/task nodes, and agent');

  // Agent-side conversations (includes messages, plans, tasks, tool calls).
  // Derive the user-side mirror paths from each chat's human_sender_path so
  // conversations from earlier --keep runs are removed too.
  const chats = await adminSql(
    `SELECT path, properties FROM 'ai' WHERE CHILD_OF($1)`,
    [`${AGENT_PATH}/inbox/chats`],
  );
  const mirrorPaths = new Set(createdConversations);
  for (const row of chats) {
    const id = row.path.split('/').pop();
    const senderPath = row.properties?.human_sender_path;
    if (senderPath && id) mirrorPaths.add(`${senderPath}/inbox/chats/${id}`);
    await deleteNode('ai', row.path);
  }

  // User-side mirrored conversations
  for (const convoPath of mirrorPaths) {
    await deleteNode('raisin:access_control', convoPath);
  }

  // Agent home (outbox messages etc.) + agent definition
  await deleteNode('ai', AGENT_PATH);
  await deleteNode('functions', AGENT_PATH);

  // Verify
  const leftover = await adminSql(
    `SELECT path FROM 'ai' WHERE DESCENDANT_OF($1) OR path = $1`,
    [AGENT_PATH],
  );
  console.log(`   leftover ai nodes under ${AGENT_PATH}: ${leftover.length}`);
  const fnLeft = await adminSql(
    `SELECT path FROM 'functions' WHERE path = $1`,
    [AGENT_PATH],
  );
  console.log(`   leftover functions nodes: ${fnLeft.length}`);
}

// ───────────────────────── main ─────────────────────────

const SCENARIOS = {
  manual: scenarioManual,
  approve_then_auto: scenarioApproveThenAuto,
  step_by_step: scenarioStepByStep,
  automatic: scenarioAutomatic,
  disabled: scenarioCreationDisabled,
};

async function main() {
  console.log(`Plan-modes e2e test against ${WS_URL} (repo: ${REPO})`);

  await adminLogin();
  await deployAgent();

  const cost0 = await costSummary();

  const client = new RaisinClient(WS_URL, {
    tokenStorage: new MemoryTokenStorage(),
    tenantId: TENANT,
    requestTimeout: 30_000,
  });
  const user = await client.loginWithEmail(PLANNER.email, PLANNER.password, REPO);
  say('setup', `Logged in as ${user.email}`);
  const db = client.database(REPO);

  const toRun = ONLY_MODE ? { [ONLY_MODE]: SCENARIOS[ONLY_MODE] } : SCENARIOS;
  if (ONLY_MODE && !SCENARIOS[ONLY_MODE]) {
    throw new Error(`Unknown --mode ${ONLY_MODE}. Options: ${Object.keys(SCENARIOS).join(', ')}`);
  }

  const results = {};
  for (const [name, fn] of Object.entries(toRun)) {
    try {
      await fn(db);
      results[name] = 'PASS';
    } catch (err) {
      results[name] = `FAIL: ${err.message}`;
      failures++;
      console.error(`\n[${name}] FAILED:`, err.message);
    }
  }

  const cost1 = await costSummary();
  say('cost', `LLM calls: ${cost1.calls - cost0.calls}, tokens: ${cost1.tokens - cost0.tokens}`);

  if (!KEEP) {
    try { await cleanup(db); }
    catch (err) { console.error('[cleanup] failed:', err.message); }
  } else {
    say('cleanup', 'Skipped (--keep)');
  }

  console.log('\n──────── RESULTS ────────');
  for (const [name, result] of Object.entries(results)) {
    console.log(`  ${result === 'PASS' ? '✅' : '❌'} ${name}: ${result}`);
  }

  client.disconnect?.();
  process.exit(failures > 0 ? 1 : 0);
}

main().catch((err) => {
  console.error('\nFATAL:', err);
  process.exit(1);
});
