#!/usr/bin/env node
/**
 * Shiftboard demo - agent<->staff chat coordination (negotiation) test.
 *
 * Proves the "agent coordinates with staff" scenario end to end with three
 * real users on separate SDK sessions:
 *
 *   1. The MANAGER asks the shift-planner agent to fill the open Saturday
 *      morning shift - via chat first, assign only after a confirmation.
 *   2. The agent messages ONE available staff member (message-staff tool ->
 *      agent outbox -> process-chat delivery -> staff inbox conversation).
 *   3. That staff member DECLINES; the reply lands in the agent's inbox and
 *      re-triggers the agent, which asks the NEXT candidate.
 *   4. The second staff member ACCEPTS; the agent assigns the shift
 *      (assign-shift) and notifies the manager (message-staff).
 *
 * The test is order-agnostic: it detects WHICH staff user the agent asked
 * first (Anna or Cara) and replies accordingly.
 *
 * Costs real Groq tokens (~6 LLM turns on llama-3.3-70b) - run sparingly.
 *
 * Prereqs: dev server on RAISIN_URL, repo set up (the test re-runs setup.mjs
 * idempotently to deploy the latest tools/agent + staff users).
 *
 * Run: npm run negotiation-test            (defaults: repo 'shiftboard')
 *      RAISIN_REPO=shiftboard2 npm run negotiation-test
 */

import { execFileSync } from 'node:child_process';
import { RaisinClient, MemoryTokenStorage } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard';
// Tenant-less /ws route by default; /sys/{tenant}/{repo} for non-default tenants
const WS_URL =
  process.env.RAISIN_WS_URL ??
  (TENANT === 'default'
    ? `${BASE_URL.replace(/^http/, 'ws')}/ws/${REPO}`
    : `${BASE_URL.replace(/^http/, 'ws')}/sys/${TENANT}/${REPO}`);
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const MANAGER = { email: 'planner@example.com', password: 'Planner12345!' };
const STAFF = [
  { name: 'Anna', email: 'anna@example.com', password: 'Staff12345!' },
  { name: 'Cara', email: 'cara@example.com', password: 'Staff12345!' },
];

const SHIFT_PATH = '/shifts/sat-morning';
const SHIFT_WORDS = /sat(urday)?[\s-]*morning|saturday/i;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function say(tag, text) {
  console.log(`\n[${tag}] ${text}`);
}

function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
}

/** Poll fn() until it returns a truthy value or timeoutMs elapses. */
async function pollFor(label, fn, timeoutMs = 60_000, intervalMs = 2_000) {
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

async function loginClient(email, password) {
  const client = new RaisinClient(WS_URL, {
    tokenStorage: new MemoryTokenStorage(),
    tenantId: TENANT,
    requestTimeout: 30_000,
  });
  const user = await client.loginWithEmail(email, password, REPO);
  return { client, db: client.database(REPO), user };
}

/** All messages across all of a user's conversations, keyed by node path. */
async function snapshotMessages(db) {
  const seen = new Map(); // path -> { conversationPath, message }
  const convos = await db.conversations.list();
  for (const convo of convos) {
    const messages = await db.conversations.getMessages(convo.conversationPath);
    for (const m of messages) {
      if (m.path) seen.set(m.path, { conversationPath: convo.conversationPath, message: m });
    }
  }
  return seen;
}

/** Find a NEW assistant message (not in baseline) matching `pattern`. */
async function findNewAgentMessage(db, baseline, pattern) {
  const current = await snapshotMessages(db);
  for (const [path, entry] of current) {
    if (baseline.has(path)) continue;
    const m = entry.message;
    if (m.role === 'assistant' && pattern.test(m.content || '')) {
      return entry;
    }
  }
  return null;
}

async function replyInConversation(db, conversationPath, text) {
  // Fire-and-forget: the agent's reaction is observed by polling, not SSE.
  for await (const _event of db.conversations.sendMessage(conversationPath, text, {
    stream: false,
  })) {
    break;
  }
}

async function main() {
  console.log(`Shiftboard negotiation test against ${WS_URL} (repo: ${REPO})`);

  // ── (a) Idempotent setup: staff users + latest tools/agent ──────────────
  say('setup', 'Running setup.mjs (idempotent redeploy: tools, agent, users)...');
  execFileSync('node', [new URL('./setup.mjs', import.meta.url).pathname], {
    stdio: 'inherit',
    env: { ...process.env, RAISIN_URL: BASE_URL, RAISIN_REPO: REPO, RAISIN_TENANT: TENANT },
  });

  // Admin session for board resets + assertions
  const adminClient = new RaisinClient(WS_URL, {
    tokenStorage: new MemoryTokenStorage(),
    tenantId: TENANT,
  });
  await adminClient.connect();
  await adminClient.authenticate({ username: ADMIN_USER, password: ADMIN_PASSWORD });
  const adminDb = adminClient.database(REPO);

  // ── (b) Reset the Saturday morning shift to open/unassigned ─────────────
  await adminDb.executeSql(
    `UPDATE 'staffing' SET properties = jsonb_set(jsonb_set(properties, '{status}', '"open"'::jsonb), '{assignee}', 'null'::jsonb) WHERE path = $1`,
    [SHIFT_PATH],
  );
  say('setup', `${SHIFT_PATH} reset to open/unassigned`);

  // Three separate user sessions
  const manager = await loginClient(MANAGER.email, MANAGER.password);
  const staffSessions = [];
  for (const s of STAFF) {
    staffSessions.push({ ...s, ...(await loginClient(s.email, s.password)) });
  }
  say('setup', `Logged in: manager + ${staffSessions.map((s) => s.name).join(' + ')}`);

  // Staff baselines BEFORE the manager kicks things off
  for (const s of staffSessions) s.baseline = await snapshotMessages(s.db);

  // ── (c) Manager asks the agent to fill the shift ─────────────────────────
  const managerConvo = await manager.db.conversations.create({
    participant: '/agents/shift-planner',
    subject: 'Fill Saturday morning',
  });
  const managerRequest =
    'Please fill the Saturday morning shift. Ask the staff via chat first - only assign someone who confirms.';
  say('manager -> agent', managerRequest);

  let agentAck = '';
  const timeout = AbortSignal.timeout(120_000);
  for await (const event of manager.db.conversations.sendMessage(
    managerConvo.conversationPath,
    managerRequest,
    { stream: true, signal: timeout },
  )) {
    if (event.type === 'text_chunk') agentAck += event.text ?? '';
    if (event.type === 'tool_call_started') {
      console.log(`    [agent tool] ${event.functionName}(${JSON.stringify(event.arguments ?? {}).slice(0, 140)})`);
    }
    if (event.type === 'failed') throw new Error(`Manager turn failed: ${JSON.stringify(event)}`);
    if (event.type === 'done' || event.type === 'waiting') break;
  }
  say('agent -> manager', agentAck.trim() || '(no text reply - tools only)');

  // Refresh the manager baseline so the initial acknowledgement does not
  // count as the final confirmation in step (i).
  const managerBaselineAfterAck = await snapshotMessages(manager.db);

  // ── (d) Which staff member did the agent ask? ────────────────────────────
  const firstAsk = await pollFor(
    'agent chat request to a staff member',
    async () => {
      for (const s of staffSessions) {
        const hit = await findNewAgentMessage(s.db, s.baseline, SHIFT_WORDS);
        if (hit) return { staff: s, ...hit };
      }
      return null;
    },
    60_000,
  );
  say(`agent -> ${firstAsk.staff.name}`, firstAsk.message.content);
  const firstStaff = firstAsk.staff;
  const secondStaff = staffSessions.find((s) => s !== firstStaff);

  // ── (e) First staff member declines ──────────────────────────────────────
  const declineText = "Sorry, I can't make it this weekend.";
  say(`${firstStaff.name} -> agent`, declineText);
  // Refresh baseline first so the agent's later reply in this thread is "new"
  firstStaff.baseline = await snapshotMessages(firstStaff.db);
  await replyInConversation(firstStaff.db, firstAsk.conversationPath, declineText);

  // ── (f) Agent asks the next candidate ────────────────────────────────────
  const secondAsk = await pollFor(
    `agent chat request to ${secondStaff.name}`,
    () => findNewAgentMessage(secondStaff.db, secondStaff.baseline, SHIFT_WORDS),
    60_000,
  );
  say(`agent -> ${secondStaff.name}`, secondAsk.message.content);

  // ── (g) Second staff member accepts ──────────────────────────────────────
  const acceptText = 'Yes, that works for me!';
  say(`${secondStaff.name} -> agent`, acceptText);
  secondStaff.baseline = await snapshotMessages(secondStaff.db);
  await replyInConversation(secondStaff.db, secondAsk.conversationPath, acceptText);

  // ── (h) Shift must be assigned to the accepter ────────────────────────────
  const shift = await pollFor(
    `${SHIFT_PATH} assigned to ${secondStaff.name}`,
    async () => {
      const res = await adminDb.executeSql(
        "SELECT path, properties FROM 'staffing' WHERE path = $1",
        [SHIFT_PATH],
      );
      const props = res.rows?.[0]?.properties ?? {};
      const assignee = (props.assignee ?? '').toString().toLowerCase();
      return props.status === 'filled' && assignee.includes(secondStaff.name.toLowerCase())
        ? props
        : null;
    },
    90_000,
  );
  say('board', `${SHIFT_PATH} -> status=${shift.status}, assignee=${shift.assignee}`);
  assert(shift.status === 'filled', 'shift status must be filled');

  // ── (i) Manager must get an agent message about the outcome ──────────────
  const managerNote = await pollFor(
    'agent confirmation to the manager',
    () =>
      findNewAgentMessage(
        manager.db,
        managerBaselineAfterAck,
        new RegExp(`${secondStaff.name}|assigned|confirmed`, 'i'),
      ),
    60_000,
  );
  say('agent -> manager', managerNote.message.content);

  console.log('\n🎉 Negotiation test PASSED:');
  console.log(`   1. manager asked the agent to fill ${SHIFT_PATH}`);
  console.log(`   2. agent asked ${firstStaff.name} via chat`);
  console.log(`   3. ${firstStaff.name} declined -> agent asked ${secondStaff.name}`);
  console.log(`   4. ${secondStaff.name} accepted -> shift assigned to ${shift.assignee}`);
  console.log('   5. manager received the confirmation message');

  await manager.client.disconnect?.();
  for (const s of staffSessions) await s.client.disconnect?.();
  await adminClient.disconnect?.();
  process.exit(0);
}

main().catch((err) => {
  console.error('\n❌ Negotiation test FAILED:', err.message || err);
  process.exit(1);
});
