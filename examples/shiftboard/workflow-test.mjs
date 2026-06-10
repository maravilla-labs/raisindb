#!/usr/bin/env node
/**
 * Shiftboard demo - durable WORKFLOW coordination test (tutorial part 5).
 *
 * Proves the /flows/fill-shift raisin:Flow end to end - the same
 * coordination as the chat-agent scenario (negotiation-test.mjs) but
 * DURABLE: the workflow engine asks staff via INBOX APPROVAL TASKS
 * (accept/decline buttons with a deadline), assigns the first accepter,
 * and reports the outcome to the manager. No LLM involved on this path -
 * the engine drives everything, so the test is deterministic and free.
 *
 *   1. Reset /shifts/sun-evening to open. Temporarily make Anna available
 *      on Sunday so the flow has TWO reachable candidates (Anna + Cara) -
 *      restored at the end.
 *   2. Start /flows/fill-shift directly via the FlowClient (the same
 *      engine path the agent's start-shift-fill tool uses).
 *   3. Anna (first candidate) gets an approval task in her inbox ->
 *      she DECLINES via the inbox API.
 *   4. Cara (next candidate) gets the task -> she ACCEPTS.
 *   5. The flow completes: shift assigned to Cara + status filled, and the
 *      manager (planner@example.com) receives the outcome chat message.
 *
 * Prereqs: dev server on RAISIN_URL, repo set up (the test re-runs
 * setup.mjs idempotently to deploy the latest functions + flow + users).
 *
 * Run: npm run workflow-test                  (defaults: repo 'shiftboard')
 *      RAISIN_REPO=shiftboard2 npm run workflow-test
 */

import { execFileSync } from 'node:child_process';
import { RaisinHttpClient, FlowClient, InboxApi } from '@raisindb/client';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';

const STAFF_PASSWORD = 'Staff12345!';
const ANNA = 'anna@example.com';
const CARA = 'cara@example.com';
const PLANNER_HOME = '/users/internal/planner-at-example-com';

const SHIFT_PATH = '/shifts/sun-evening';
const FLOW_PATH = '/flows/fill-shift';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function say(tag, text) {
  console.log(`\n[${tag}] ${text}`);
}

function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
}

/** Poll fn() until it returns a truthy value or timeoutMs elapses. */
async function pollFor(label, fn, timeoutMs = 60_000, intervalMs = 1_500) {
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

/** Identity-user login -> { token, home, inbox: InboxApi }. */
async function loginStaff(email) {
  const res = await fetch(`${BASE_URL}/auth/${REPO}/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password: STAFF_PASSWORD }),
  });
  if (!res.ok) {
    throw new Error(`identity login failed for ${email}: HTTP ${res.status} ${await res.text()}`);
  }
  const tokens = await res.json();
  const token = tokens.access_token;
  // InboxApi only needs getAccessToken() from the auth manager
  const inbox = new InboxApi(BASE_URL, REPO, { getAccessToken: () => token });
  return { email, token, home: tokens.identity?.home ?? null, inbox };
}

/** Find this flow instance's pending approval task in a user's inbox. */
async function findTask(staff, instanceId, timeoutMs = 90_000) {
  return pollFor(
    `pending inbox task for ${staff.email} (instance ${instanceId})`,
    async () => {
      const { tasks } = await staff.inbox.listTasks({ status: 'pending' });
      return tasks.find((t) => t.flow_instance_id === instanceId) ?? null;
    },
    timeoutMs,
  );
}

/**
 * Complete a task and make sure the flow actually leaves the human-task
 * wait. Recovers from a known engine race where the queued resume job is
 * dropped ("Missing job context") by re-issuing an admin resume.
 */
async function completeAndResume(staff, flows, instanceId, task, response) {
  const result = await staff.inbox.completeTask(task.id, response);
  say(staff.email, `task completed (${response.action}) -> resume job ${result.flow?.job_id ?? '?'}`);

  try {
    await pollFor(
      'flow to leave the completed task',
      async () => {
        const status = await flows.getInstanceStatus(instanceId);
        if (['completed', 'failed', 'rolled_back', 'cancelled'].includes(status.status)) return status;
        // Still waiting on the SAME task means the resume job was dropped
        const { tasks } = await staff.inbox.listTasks({ status: 'pending' });
        return tasks.some((t) => t.id === task.id) ? null : status;
      },
      20_000,
    );
  } catch {
    say('recovery', 'flow still waiting on the completed task - re-issuing admin resume');
    await flows.resume(instanceId, { ...response, completed_by: staff.email, task_path: task.path });
  }
}

async function main() {
  console.log(`Shiftboard workflow test against ${BASE_URL} (repo: ${REPO})`);

  // ── (a) Idempotent setup: functions, flow, agent, users ─────────────────
  say('setup', 'Running setup.mjs (idempotent redeploy: tools, flow, agent, users)...');
  execFileSync('node', [new URL('./setup.mjs', import.meta.url).pathname], {
    stdio: 'inherit',
    env: { ...process.env, RAISIN_URL: BASE_URL, RAISIN_REPO: REPO, RAISIN_TENANT: TENANT },
  });

  // Admin session for board resets + assertions
  const admin = new RaisinHttpClient(BASE_URL, { tenantId: TENANT });
  await admin.authenticate({ username: ADMIN_USER, password: ADMIN_PASSWORD });
  const flows = FlowClient.fromHttpClient(admin, BASE_URL, REPO);

  // SQL via plain fetch with an explicit Bearer token. (RaisinHttpClient's
  // own request() drops the Authorization header after ADMIN auth - it
  // never learns the token expiry, so isAuthenticated() stays false and
  // queries run anonymously / RLS-filtered. FlowClient/InboxApi read the
  // token directly from the auth manager and are unaffected.)
  const adminToken = admin.getAuthManager().getAccessToken();
  const sql = async (query, params = []) => {
    const res = await fetch(`${BASE_URL}/api/sql/${REPO}`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${adminToken}`,
      },
      body: JSON.stringify({ sql: query, params }),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) {
      throw new Error(`SQL failed: HTTP ${res.status} ${JSON.stringify(body)}`);
    }
    return body;
  };

  // ── (b) Board fixture ────────────────────────────────────────────────────
  // Right after setup.mjs re-seeds the board, node writes can race the
  // freshly (re)created nodes - retry the fixture SQL briefly.
  const sqlRetry = async (query, params) => {
    let lastErr;
    for (let i = 0; i < 10; i++) {
      try {
        return await sql(query, params);
      } catch (err) {
        lastErr = err;
        await sleep(1000);
      }
    }
    throw lastErr;
  };

  // Reset the Sunday evening shift to open/unassigned
  await sqlRetry(
    `UPDATE 'staffing' SET properties = jsonb_set(jsonb_set(properties, '{status}', '"open"'::jsonb), '{assignee}', 'null'::jsonb) WHERE path = $1`,
    [SHIFT_PATH],
  );
  say('setup', `${SHIFT_PATH} reset to open/unassigned`);

  // The seed data has only ONE registered staff member available on Sunday
  // (Cara). Temporarily make Anna available on Sunday too, so the workflow
  // demonstrably asks the NEXT candidate after a decline. Restored below.
  const annaRow = await sqlRetry("SELECT properties FROM 'staffing' WHERE path = '/staff/anna'");
  const annaProps = annaRow.rows?.[0]?.properties;
  assert(annaProps, '/staff/anna must exist');
  const originalAnnaDays = annaProps.available_days ?? [];
  const patchedAnnaDays = originalAnnaDays.includes('sunday')
    ? originalAnnaDays
    : [...originalAnnaDays, 'sunday'];
  const setAnnaDays = (days) =>
    sqlRetry(
      `UPDATE 'staffing' SET properties = jsonb_set(properties, '{available_days}', $1::jsonb) WHERE path = '/staff/anna'`,
      [JSON.stringify(days)],
    );
  await setAnnaDays(patchedAnnaDays);
  say('setup', `Anna temporarily available on sunday (${patchedAnnaDays.join(', ')})`);

  let exitCode = 1;
  try {
    // Staff sessions (identity users -> inbox API)
    const anna = await loginStaff(ANNA);
    const cara = await loginStaff(CARA);
    say('setup', `Staff logged in: ${ANNA} (home ${anna.home}) + ${CARA} (home ${cara.home})`);

    const startedAt = new Date().toISOString();

    // ── (c) Start the workflow directly (engine path, no LLM) ─────────────
    const { instance_id } = await flows.run(FLOW_PATH, { shift_path: SHIFT_PATH });
    say('flow', `${FLOW_PATH} started for ${SHIFT_PATH} -> instance ${instance_id}`);

    // ── (d) Anna (first candidate) gets the approval task ─────────────────
    const annaTask = await findTask(anna, instance_id);
    say('inbox', `Anna's task: "${annaTask.title}"`);
    assert(annaTask.task_type === 'approval', `expected approval task, got ${annaTask.task_type}`);
    assert(
      /sunday evening/i.test(annaTask.title) && /15:00/.test(annaTask.title),
      `task title must name the shift and time, got: ${annaTask.title}`,
    );
    assert(
      (annaTask.description ?? '').includes(SHIFT_PATH),
      `task description must carry the shift path, got: ${annaTask.description}`,
    );
    const optionValues = (annaTask.options ?? []).map((o) => o.value);
    assert(
      optionValues.includes('accept') && optionValues.includes('decline'),
      `task must offer accept/decline, got: ${optionValues.join(', ')}`,
    );
    // Cara must NOT have been asked yet (sequential, not broadcast)
    const caraEarly = await cara.inbox.listTasks({ status: 'pending' });
    assert(
      !caraEarly.tasks.some((t) => t.flow_instance_id === instance_id),
      'Cara must not be asked before Anna answered',
    );
    say('check', 'Task shape OK; Cara not asked yet (sequential ask)');

    // ── (e) Anna declines -> the flow asks Cara ───────────────────────────
    await completeAndResume(anna, flows, instance_id, annaTask, {
      action: 'decline',
      comment: "Sorry, can't make it on Sunday.",
    });

    const caraTask = await findTask(cara, instance_id);
    say('inbox', `Cara's task: "${caraTask.title}"`);
    assert(caraTask.task_type === 'approval', 'Cara must get an approval task');
    assert(caraTask.id !== annaTask.id, 'Cara must get her OWN task');

    // ── (f) Cara accepts -> flow assigns + completes ───────────────────────
    await completeAndResume(cara, flows, instance_id, caraTask, {
      action: 'accept',
      comment: 'Sure, I can take it.',
    });

    const final = await pollFor(
      'flow instance to complete',
      async () => {
        const status = await flows.getInstanceStatus(instance_id);
        return ['completed', 'failed', 'rolled_back', 'cancelled'].includes(status.status)
          ? status
          : null;
      },
      90_000,
    );
    assert(
      final.status === 'completed',
      `flow must complete, got: ${final.status} ${final.error ?? ''}`,
    );
    const outputs = final.variables?.step_outputs ?? {};
    say('flow', `completed; resolve_accepter -> ${JSON.stringify(outputs.resolve_accepter ?? {})}`);
    assert(outputs.pick_candidates?.candidates?.length === 2, 'flow must have had 2 candidates');
    assert(outputs.resolve_accepter?.accepted === true, 'resolve_accepter must report an accept');
    assert(
      outputs.resolve_accepter?.accepter_name === 'Cara',
      `accepter must be Cara, got ${outputs.resolve_accepter?.accepter_name}`,
    );
    assert(
      (outputs.resolve_accepter?.declined_names ?? []).includes('Anna'),
      'Anna must be recorded as declined',
    );
    assert(
      outputs.assign_shift?.assignee === 'Cara',
      `assign step must have assigned Cara, got ${JSON.stringify(outputs.assign_shift)}`,
    );
    assert(
      outputs.notify_manager?.delivered_to === 'planner@example.com',
      `manager notification must have been sent, got ${JSON.stringify(outputs.notify_manager)}`,
    );

    // ── (g) Board state: shift filled by Cara ──────────────────────────────
    const shiftRes = await sql("SELECT properties FROM 'staffing' WHERE path = $1", [SHIFT_PATH]);
    const shiftProps = shiftRes.rows?.[0]?.properties ?? {};
    say('board', `${SHIFT_PATH} -> status=${shiftProps.status}, assignee=${shiftProps.assignee}`);
    assert(shiftProps.status === 'filled', 'shift must be filled');
    assert(shiftProps.assignee === 'Cara', `shift must be assigned to Cara, got ${shiftProps.assignee}`);

    // ── (h) Manager got the outcome message ───────────────────────────────
    // message-staff drops the outcome into the agent outbox; the messaging
    // pipeline delivers it into the planner's inbox conversation (the
    // planner's user home in raisin:access_control).
    const managerMsg = await pollFor(
      'outcome message in the planner inbox',
      async () => {
        const res = await sql(
          `SELECT path, properties FROM 'raisin:access_control' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:Message'`,
          [`${PLANNER_HOME}/inbox/chats`],
        );
        for (const row of res.rows ?? []) {
          const body = row.properties?.body ?? {};
          const text = body.message_text ?? body.content ?? '';
          const createdAt = row.properties?.created_at ?? '';
          if (createdAt >= startedAt && /Cara/.test(text) && /accept/i.test(text)) {
            return text;
          }
        }
        return null;
      },
      60_000,
    );
    say('manager inbox', managerMsg);
    assert(/Sunday Evening/i.test(managerMsg), 'outcome message must name the shift');

    console.log('\n🎉 Workflow test PASSED:');
    console.log(`   1. ${FLOW_PATH} started for ${SHIFT_PATH} (2 candidates)`);
    console.log('   2. Anna got an inbox approval task and DECLINED');
    console.log('   3. the engine asked Cara next - she ACCEPTED');
    console.log('   4. flow completed: shift assigned to Cara, status filled');
    console.log('   5. manager received the outcome message');
    exitCode = 0;
  } finally {
    // Restore the demo data: Anna's original availability. The shift stays
    // assigned to Cara (a filled sun-evening is valid demo state).
    await setAnnaDays(originalAnnaDays).catch((e) =>
      console.error('⚠️  failed to restore /staff/anna available_days:', e.message),
    );
    say('cleanup', `Anna's availability restored (${originalAnnaDays.join(', ')})`);
  }
  process.exit(exitCode);
}

main().catch((err) => {
  console.error('\n❌ Workflow test FAILED:', err.message || err);
  process.exit(1);
});
