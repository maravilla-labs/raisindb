#!/usr/bin/env node
/**
 * Headless end-to-end proof of the Planner tab: PLANS + WORKFLOWS composed.
 *
 * Flow: reset all shifts to open → login as the manager → Planner tab →
 * "Fill all open weekend shifts" → the plan-enabled coordinator agent
 * (/agents/shift-coordinator, approve_then_auto) proposes a plan with ONE
 * task per open shift → Approve → each task starts the durable
 * /flows/fill-shift workflow (task completed = workflow STARTED, the honest
 * seam) → staff really have pending inbox approval tasks (verified via the
 * inbox API as anna/cara) → ONE staff task is accepted via the API → that
 * shift flips to filled on the board LIVE (node subscription, no reload).
 *
 * The same staged run also captures tutorial screenshots into
 * /tmp/shots/planner-tutorial/ (planner-tab, plan-proposal, plan-toolcalls,
 * plan-running, plan-complete, staff-inbox-task, board-filled).
 *
 * Costs ONE real Groq run (budget asserted ≤ 15 LLM calls, from
 * raisin:AICostRecord). Restores demo state afterwards: leftover waiting
 * flow instances are cancelled, their inbox tasks removed, and the board is
 * re-seeded to all-open.
 *
 * Prereqs:
 *   - raisin-server on RAISIN_URL (default http://localhost:8081) with the
 *     shiftboard setup applied (RAISIN_REPO=shiftboard2 node setup.mjs) and
 *     Groq configured for the tenant
 *   - the frontend running on APP_URL (default http://localhost:5175,
 *     `npm run build && npm run start` in frontend/)
 *   - Playwright in PLAYWRIGHT_DIR (default /tmp/shots/node_modules)
 *
 * IMPORTANT: never wait on networkidle — the chat SSE subscription keeps the
 * network busy forever.
 *
 * Run: RAISIN_REPO=shiftboard2 node planner-tab-check.mjs
 */
import { mkdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard2';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const APP_URL = process.env.APP_URL ?? 'http://localhost:5175';
const PLAYWRIGHT_DIR = process.env.PLAYWRIGHT_DIR ?? '/tmp/shots/node_modules';
const SHOTS_DIR = process.env.SHOTS_DIR ?? '/tmp/shots/planner-tutorial';

const AGENT_HOME = '/agents/shift-coordinator';
const STAFF = ['anna@example.com', 'cara@example.com'];
const STAFF_PASSWORD = 'Staff12345!';
const CHAT_PROMPT = 'Fill all open weekend shifts';

const require = createRequire(path.join(PLAYWRIGHT_DIR, 'noop.js'));
const { chromium } = require('playwright');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...args) => console.log('[planner-check]', ...args);

let failures = 0;
function assert(cond, label) {
  if (cond) {
    log('PASS:', label);
  } else {
    failures += 1;
    console.error('[planner-check] FAIL:', label);
  }
}

// ---------------------------------------------------------------------------
// Server-side helpers (plain HTTP, mirrors setup.mjs / taskpilot check.mjs)
// ---------------------------------------------------------------------------

let adminToken = null;

async function adminLogin() {
  const res = await fetch(`${BASE_URL}/api/raisindb/sys/${TENANT}/auth`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: ADMIN_USER, password: ADMIN_PASSWORD }),
  });
  if (!res.ok) throw new Error(`admin login failed: HTTP ${res.status}`);
  adminToken = (await res.json()).token;
}

async function adminSql(sql, params = []) {
  const res = await fetch(`${BASE_URL}/api/sql/${REPO}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${adminToken}` },
    body: JSON.stringify({ sql, params }),
  });
  if (!res.ok) throw new Error(`sql failed: HTTP ${res.status} ${await res.text()}`);
  return (await res.json()).rows ?? [];
}

/** Reset every shift to open/unassigned (deterministic run + demo re-seed). */
async function resetShiftsOpen() {
  const rows = await adminSql(
    `SELECT path FROM 'staffing' WHERE CHILD_OF($1) ORDER BY path ASC`,
    ['/shifts'],
  );
  for (const row of rows) {
    await adminSql(
      `UPDATE 'staffing' SET properties = jsonb_set(jsonb_set(properties, '{status}', '"open"'::jsonb), '{assignee}', 'null'::jsonb) WHERE path = $1`,
      [row.path],
    );
  }
  log(`reset ${rows.length} shifts to open/unassigned`);
  return rows.map((r) => r.path);
}

/**
 * Delete previous coordinator conversations (agent side + user mirror) so
 * the run is deterministic: the app starts a fresh conversation and exactly
 * one plan card exists. SQL DELETE — the HTTP DELETE endpoint does not
 * reliably remove nodes (see plan-modes-test.mjs).
 */
async function cleanupConversations() {
  const chats = await adminSql(`SELECT path, properties FROM 'ai' WHERE CHILD_OF($1)`, [
    `${AGENT_HOME}/inbox/chats`,
  ]);
  for (const row of chats) {
    const id = row.path.split('/').pop();
    const senderPath = row.properties?.human_sender_path;
    await adminSql(`DELETE FROM 'ai' WHERE DESCENDANT_OF($1)`, [row.path]).catch(() => {});
    await adminSql(`DELETE FROM 'ai' WHERE path = $1`, [row.path]).catch(() => {});
    if (senderPath && id) {
      const mirror = `${senderPath}/inbox/chats/${id}`;
      await adminSql(`DELETE FROM 'raisin:access_control' WHERE DESCENDANT_OF($1)`, [mirror]).catch(() => {});
      await adminSql(`DELETE FROM 'raisin:access_control' WHERE path = $1`, [mirror]).catch(() => {});
    }
  }
  if (chats.length > 0) log(`cleaned up ${chats.length} previous coordinator conversation(s)`);
}

/**
 * Tutorial-quality screenshots must not show glitch bubbles ("could not
 * generate a complete response" etc.). Best effort: find such messages in
 * the coordinator conversation (agent side + user mirror) and delete them.
 * Returns true when something was removed (caller should reload the page).
 */
async function scrubGlitchMessages() {
  const GLITCH = /could not generate|unable to generate a complete response/i;
  let removed = 0;
  for (const [ws, root] of [
    ['ai', `${AGENT_HOME}/inbox/chats`],
    ['raisin:access_control', '/users/internal/planner-at-example-com/inbox/chats'],
  ]) {
    const rows = await adminSql(
      `SELECT path, properties FROM '${ws}' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:Message'`,
      [root],
    ).catch(() => []);
    for (const row of rows) {
      const text = JSON.stringify(row.properties ?? {});
      if (GLITCH.test(text)) {
        await adminSql(`DELETE FROM '${ws}' WHERE path = $1`, [row.path]).catch(() => {});
        removed += 1;
      }
    }
  }
  if (removed > 0) log(`scrubbed ${removed} glitch message node(s)`);
  return removed > 0;
}

async function costSummary() {
  const rows = await adminSql(
    `SELECT properties FROM 'ai' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
    [AGENT_HOME],
  );
  let tokens = 0;
  for (const r of rows) tokens += Number(r.properties?.total_tokens ?? 0) || 0;
  return { calls: rows.length, tokens };
}

// ---------------------------------------------------------------------------
// Staff inbox (identity users) + flow instance admin
// ---------------------------------------------------------------------------

async function loginStaff(email) {
  const res = await fetch(`${BASE_URL}/auth/${REPO}/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email, password: STAFF_PASSWORD }),
  });
  if (!res.ok) throw new Error(`staff login failed for ${email}: HTTP ${res.status}`);
  const tokens = await res.json();
  return { email, token: tokens.access_token };
}

async function staffPendingTasks(staff) {
  const res = await fetch(`${BASE_URL}/api/inbox/${REPO}?status=pending`, {
    headers: { authorization: `Bearer ${staff.token}` },
  });
  if (!res.ok) throw new Error(`inbox list failed for ${staff.email}: HTTP ${res.status}`);
  return (await res.json()).tasks ?? [];
}

async function staffCompleteTask(staff, taskId, action) {
  const res = await fetch(`${BASE_URL}/api/inbox/${REPO}/tasks/${taskId}/complete`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${staff.token}` },
    body: JSON.stringify({ response: { action } }),
  });
  if (!res.ok) throw new Error(`complete task failed: HTTP ${res.status} ${await res.text()}`);
  return res.json();
}

async function cancelFlow(instanceId) {
  const res = await fetch(`${BASE_URL}/api/flows/${REPO}/instances/${instanceId}/cancel`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${adminToken}` },
  });
  return res.ok;
}

/**
 * Restore demo state: cancel waiting fill-shift instances still parked on a
 * staff inbox task, delete the leftover pending task nodes, re-seed the
 * board to all-open.
 */
async function restoreDemoState(staffSessions) {
  let cancelled = 0;
  for (const staff of staffSessions) {
    const tasks = await staffPendingTasks(staff).catch(() => []);
    for (const task of tasks) {
      if (task.flow_instance_id) {
        const ok = await cancelFlow(task.flow_instance_id);
        if (ok) cancelled += 1;
      }
      if (task.path) {
        await adminSql(`DELETE FROM 'raisin:access_control' WHERE path = $1`, [task.path]).catch(() => {});
      }
    }
  }
  await resetShiftsOpen();
  log(`demo state restored (cancelled ${cancelled} waiting flow instance(s), board re-seeded to all open)`);
}

// ---------------------------------------------------------------------------
// Browser helpers
// ---------------------------------------------------------------------------

const shot = async (page, name) => {
  await page.screenshot({ path: path.join(SHOTS_DIR, name) });
  log(`📸 ${name}`);
};

async function loginApp(page, email, password) {
  await page.goto(APP_URL, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.login-card, .app', { timeout: 15000 });
  if (await page.locator('.login-card').count()) {
    if (email) {
      await page.fill('.login-card input[name="email"]', email);
      await page.fill('.login-card input[name="password"]', password);
    }
    await page.click('.login-card button[type="submit"]');
  }
  await page.waitForSelector('[data-testid="tab-planner"]', { timeout: 20000 });
}

async function openPlannerTab(page) {
  await page.click('[data-testid="tab-planner"]');
  await page.waitForSelector('[data-testid="side-planner"]', { timeout: 10000 });
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

async function main() {
  mkdirSync(SHOTS_DIR, { recursive: true });
  await adminLogin();

  // Deterministic fixture: all shifts open, no previous coordinator chats,
  // no stale waiting flows from earlier runs.
  const staffSessions = [];
  for (const email of STAFF) staffSessions.push(await loginStaff(email));
  await restoreDemoState(staffSessions); // also resets the shifts
  await cleanupConversations();
  const shiftPaths = await adminSql(
    `SELECT path FROM 'staffing' WHERE CHILD_OF($1) AND properties->>'status'::String = $2 ORDER BY path ASC`,
    ['/shifts', 'open'],
  ).then((rows) => rows.map((r) => r.path));
  log(`open shifts: ${shiftPaths.length} (${shiftPaths.join(', ')})`);
  const cost0 = await costSummary();

  const browser = await chromium.launch({ headless: true });
  let acceptedShift = null;
  let planTaskTitles = [];
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await context.newPage();
    page.on('pageerror', (err) => console.error('[browser] PAGEERROR:', err.message));

    // 1. Login (prefilled manager creds) -------------------------------------
    await loginApp(page);
    assert(true, 'login → app shell with Board | Planner tabs');

    // 2. Planner tab: coordinator chat + plan panel, board stays visible -----
    await openPlannerTab(page);
    const boardVisible = await page.locator('[data-testid="shift-card"]').count();
    assert(boardVisible === 5, `board stays visible next to the Planner view (${boardVisible} cards)`);
    await sleep(1200); // let the fresh (empty) chat settle for the screenshot
    await shot(page, 'planner-tab.png');

    // 3. Ask for the weekend (ONE Groq run starts here) -----------------------
    const input = page.locator('[data-testid="planner-chat-input"]');
    await input.fill(CHAT_PROMPT);
    await input.press('Enter');
    await page.waitForSelector('[data-testid="planner-chat-bubble-user"]', { timeout: 10000 });
    assert(true, 'manager message appears in the coordinator chat');

    // 4. Plan proposal: pending_approval, one task per open shift -------------
    // While waiting, capture the tool-call chips of the proposal turn
    // (list-shifts / create-plan) as the plan-toolcalls fallback shot.
    const proposal = page.locator('[data-testid="plan-card"][data-status="pending_approval"]');
    let toolShot = false;
    {
      const deadline = Date.now() + 150000;
      while (Date.now() < deadline) {
        if (!toolShot && (await page.locator('.tool-badge').count()) > 0) {
          await shot(page, 'plan-toolcalls.png');
          toolShot = true;
        }
        if (await proposal.count()) break;
        await sleep(200);
      }
    }
    await proposal.waitFor({ timeout: 5000 });
    assert(true, 'plan card appears with status pending_approval');
    await proposal.locator('[data-testid="plan-approve"]').waitFor({ timeout: 15000 });
    const taskCount = await proposal.locator('[data-testid="task-row"]').count();
    assert(
      taskCount === shiftPaths.length,
      `plan proposes one task per open shift (${taskCount}/${shiftPaths.length})`,
    );
    planTaskTitles = await proposal.locator('[data-testid="task-row"]').allInnerTexts();
    const titlesCoverShifts = shiftPaths.every((p) => planTaskTitles.some((t) => t.includes(p)));
    assert(titlesCoverShifts, `every task title carries its shift path (${JSON.stringify(planTaskTitles)})`);
    // Streaming settled before the proposal screenshot (no half-rendered text)
    await sleep(1500);
    await shot(page, 'plan-proposal.png');

    // 5. Approve → every task runs start-shift-fill ---------------------------
    await proposal.locator('[data-testid="plan-approve"]').click();
    log('plan approved — tasks now start one fill-shift workflow each…');
    {
      let runningShot = false;
      let execToolShot = false;
      const deadline = Date.now() + 300000;
      while (Date.now() < deadline) {
        // Execution-time tool chips (start-shift-fill) are the preferred
        // plan-toolcalls shot — overwrite the proposal-phase fallback once.
        if (!execToolShot && (await page.locator('.tool-badge').count())) {
          await shot(page, 'plan-toolcalls.png');
          toolShot = true;
          execToolShot = true;
        }
        const completed = await page
          .locator('[data-testid="task-row"][data-status="completed"]')
          .count();
        const total = await page.locator('[data-testid="task-row"]').count();
        if (!runningShot && completed > 0 && completed < total) {
          await shot(page, 'plan-running.png');
          runningShot = true;
        }
        if (await page.locator('[data-testid="plan-card"][data-status="completed"]').count()) break;
        await sleep(150);
      }
      if (!runningShot) {
        // Mid-state was too fast to catch — the closest equivalent is the
        // moment right after completion flips (tasks done, summary pending).
        await shot(page, 'plan-running.png');
      }
    }
    await page.waitForSelector('[data-testid="plan-card"][data-status="completed"]', {
      timeout: 60000,
    });
    const doneTasks = await page
      .locator('[data-testid="task-row"][data-status="completed"]')
      .count();
    assert(
      doneTasks === shiftPaths.length,
      `plan completed: all ${shiftPaths.length} tasks done (= workflows STARTED, not shifts filled)`,
    );

    // 6. Honest closing summary ------------------------------------------------
    const bubbles = page.locator('[data-testid="planner-chat-bubble-assistant"]');
    let replyText = '';
    {
      const deadline = Date.now() + 60000;
      while (Date.now() < deadline) {
        if (await bubbles.count()) {
          replyText = (await bubbles.last().innerText()).trim();
          if (/workflow|started|inbox|accept/i.test(replyText)) break;
        }
        await sleep(1000);
      }
    }
    assert(
      /workflow|started|inbox|accept/i.test(replyText),
      `summary is honest about the seam (workflows started, staff decide): "${replyText.slice(0, 140)}"`,
    );
    // Tutorial shot: completed plan + summary, scrubbed of glitch bubbles.
    if (await scrubGlitchMessages()) {
      await page.reload({ waitUntil: 'domcontentloaded' });
      await page.waitForSelector('[data-testid="tab-planner"]', { timeout: 20000 });
      await openPlannerTab(page);
      await sleep(2500);
    }
    await sleep(1000);
    await shot(page, 'plan-complete.png');

    // 7. The seam, server-side: staff really have pending inbox tasks --------
    // Each fill-shift instance asks ONE candidate at a time, so right now
    // there must be exactly one pending task per started workflow across the
    // reachable staff (anna + cara).
    let pendingByStaff = [];
    {
      const deadline = Date.now() + 90000;
      for (;;) {
        pendingByStaff = [];
        for (const staff of staffSessions) {
          const tasks = (await staffPendingTasks(staff)).filter((t) => t.flow_instance_id);
          pendingByStaff.push({ staff, tasks });
        }
        const total = pendingByStaff.reduce((n, e) => n + e.tasks.length, 0);
        if (total >= shiftPaths.length || Date.now() > deadline) break;
        await sleep(2000);
      }
    }
    const allTasks = pendingByStaff.flatMap((e) => e.tasks);
    assert(
      allTasks.length === shiftPaths.length,
      `staff inboxes hold one pending approval task per workflow (got ${allTasks.length}/${shiftPaths.length})`,
    );
    const board0 = await adminSql(
      `SELECT path FROM 'staffing' WHERE CHILD_OF($1) AND properties->>'status'::String = $2`,
      ['/shifts', 'filled'],
    );
    assert(board0.length === 0, 'no shift is filled yet — plan done means workflows started, board honest');

    // Tutorial shot: Anna's inbox task panel with the accept/decline buttons.
    {
      const staffContext = await browser.newContext({ viewport: { width: 1440, height: 900 } });
      const staffPage = await staffContext.newPage();
      try {
        await loginApp(staffPage, 'anna@example.com', STAFF_PASSWORD);
        await staffPage.waitForSelector('.task-card .task-btn', { timeout: 30000 });
        await sleep(800);
        await shot(staffPage, 'staff-inbox-task.png');
      } catch (err) {
        log('staff-inbox-task screenshot failed (non-fatal):', err.message);
      } finally {
        await staffContext.close();
      }
    }

    // 8. ONE staff member accepts via the API → board fills LIVE --------------
    const entry = pendingByStaff.find((e) => e.tasks.length > 0);
    if (!entry) throw new Error('no staff member has a pending workflow task — workflows did not start');
    const task = entry.tasks[0];
    const m = (task.description ?? '').match(/\/shifts\/[a-z-]+/);
    acceptedShift = m ? m[0] : null;
    assert(!!acceptedShift, `accepted task names its shift path (${task.title})`);
    log(`${entry.staff.email} accepts "${task.title}" (${acceptedShift})`);
    await staffCompleteTask(entry.staff, task.id, 'accept');

    // Remove the accepted task from the restore set
    entry.tasks = entry.tasks.slice(1);

    // The shift card flips to filled in the OPEN PAGE (live node event /
    // board resync) — no reload.
    try {
      await page.waitForSelector(
        `[data-testid="shift-card"][data-path="${acceptedShift}"][data-status="filled"]`,
        { timeout: 45000 },
      );
      assert(true, `${acceptedShift} flipped to filled on the board live`);
    } catch {
      assert(false, `${acceptedShift} flipped to filled on the board live`);
    }
    const filledRows = await adminSql(
      `SELECT properties->>'assignee'::String AS assignee FROM 'staffing' WHERE path = $1`,
      [acceptedShift],
    );
    assert(!!filledRows[0]?.assignee, `server confirms the assignee (${filledRows[0]?.assignee})`);

    // Tutorial shot: planner view with the freshly filled shift on the board.
    await sleep(1200);
    await shot(page, 'board-filled.png');
  } finally {
    await browser.close();
  }

  const cost1 = await costSummary();
  log(`Groq spend: ${cost1.calls - cost0.calls} LLM calls, ${cost1.tokens - cost0.tokens} tokens`);
  assert(cost1.calls - cost0.calls <= 15, 'run used ≤ 15 LLM calls');
  log(`plan task titles: ${JSON.stringify(planTaskTitles)}`);

  // Restore demo state: cancel the remaining waiting workflows, drop their
  // tasks, re-open all shifts (including the one just filled).
  await restoreDemoState(staffSessions);

  if (failures > 0) {
    console.error(`[planner-check] ${failures} assertion(s) failed`);
    process.exit(1);
  }
  log('ALL CHECKS PASSED');
}

main().catch((err) => {
  console.error('[planner-check] ERROR:', err);
  process.exit(1);
});
