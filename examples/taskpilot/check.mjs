#!/usr/bin/env node
/**
 * Headless end-to-end proof for the TaskPilot demo (plan/task system through
 * the SDK React integration).
 *
 * Flow: login (prefilled demo creds) → checklist renders → ask the pilot
 * agent for a 2-task plan that marks an item done → the plan proposal card
 * appears (pending_approval, Approve/Reject) → click Approve → tasks
 * progress to completed → the checklist item actually changed on the server
 * AND in the live UI → the final reply summarizes.
 *
 * Costs ONE real Groq run (a handful of LLM calls) - run sparingly.
 * The LLM-call count before/after is reported from raisin:AICostRecord.
 *
 * Prereqs:
 *   - raisin-server on RAISIN_URL (default http://localhost:8081) with the
 *     taskpilot setup applied (node setup.mjs) and Groq configured
 *   - Playwright installed somewhere; default lookup dir
 *     /tmp/shots/node_modules (override with PLAYWRIGHT_DIR)
 *   - If no dev server is already on APP_URL, this script starts
 *     `react-router dev` itself.
 *
 * IMPORTANT: never wait on networkidle — the chat SSE subscription keeps the
 * network busy forever.
 *
 * Run: node check.mjs
 */
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const BASE_URL = process.env.RAISIN_URL ?? 'http://localhost:8081';
const REPO = process.env.RAISIN_REPO ?? 'taskpilot';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const APP_URL = process.env.APP_URL ?? 'http://localhost:5177';
const PLAYWRIGHT_DIR = process.env.PLAYWRIGHT_DIR ?? '/tmp/shots/node_modules';

const TARGET_ITEM = '/checklist/design-hero';
const CHAT_PROMPT =
  'Plan and execute: mark "Design hero section" as done, then summarize the checklist.';

const require = createRequire(path.join(PLAYWRIGHT_DIR, 'noop.js'));
const { chromium } = require('playwright');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...args) => console.log('[check]', ...args);

// ---------------------------------------------------------------------------
// Server-side helpers (plain HTTP, mirrors setup.mjs)
// ---------------------------------------------------------------------------

let adminToken = null;

async function adminSql(sql, params = []) {
  if (!adminToken) {
    const res = await fetch(`${BASE_URL}/api/raisindb/sys/${TENANT}/auth`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: ADMIN_USER, password: ADMIN_PASSWORD }),
    });
    if (!res.ok) throw new Error(`admin login failed: HTTP ${res.status}`);
    adminToken = (await res.json()).token;
  }
  const res = await fetch(`${BASE_URL}/api/sql/${REPO}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${adminToken}`,
    },
    body: JSON.stringify({ sql, params }),
  });
  if (!res.ok) throw new Error(`sql failed: HTTP ${res.status} ${await res.text()}`);
  return (await res.json()).rows ?? [];
}

/** Reset the target item to todo so the run is deterministic. */
async function resetTargetItem() {
  const rows = await adminSql(
    `SELECT path, properties FROM 'projects' WHERE path = $1`,
    [TARGET_ITEM],
  );
  if (rows.length !== 1) throw new Error(`item not found: ${TARGET_ITEM} (run setup.mjs)`);
  const props = rows[0].properties;
  if (props.status !== 'todo') {
    props.status = 'todo';
    props.notes = '';
    await adminSql(`UPDATE 'projects' SET properties = $1::jsonb WHERE path = $2`, [
      JSON.stringify(props),
      TARGET_ITEM,
    ]);
    log('reset', TARGET_ITEM, 'back to todo');
  }
}

/**
 * Delete previous demo conversations (agent side + user mirror) so the run
 * is deterministic: the app then starts a fresh conversation and exactly
 * one plan card exists. Mirrors the plan-modes-test cleanup (SQL DELETE —
 * the HTTP DELETE endpoint doesn't reliably remove nodes).
 */
async function cleanupConversations() {
  const chats = await adminSql(`SELECT path, properties FROM 'ai' WHERE CHILD_OF($1)`, [
    '/agents/pilot/inbox/chats',
  ]);
  for (const row of chats) {
    const id = row.path.split('/').pop();
    const senderPath = row.properties?.human_sender_path;
    await adminSql(`DELETE FROM 'ai' WHERE DESCENDANT_OF($1)`, [row.path]).catch(() => {});
    await adminSql(`DELETE FROM 'ai' WHERE path = $1`, [row.path]).catch(() => {});
    if (senderPath && id) {
      const mirror = `${senderPath}/inbox/chats/${id}`;
      await adminSql(`DELETE FROM 'raisin:access_control' WHERE DESCENDANT_OF($1)`, [
        mirror,
      ]).catch(() => {});
      await adminSql(`DELETE FROM 'raisin:access_control' WHERE path = $1`, [mirror]).catch(
        () => {},
      );
    }
  }
  if (chats.length > 0) log(`cleaned up ${chats.length} previous conversation(s)`);
}

async function costSummary() {
  const rows = await adminSql(
    `SELECT properties FROM 'ai'
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
    ['/agents/pilot'],
  );
  let tokens = 0;
  for (const r of rows) tokens += Number(r.properties?.total_tokens ?? 0) || 0;
  return { calls: rows.length, tokens };
}

// ---------------------------------------------------------------------------
// Dev server (reuse a running one, else start react-router dev ourselves)
// ---------------------------------------------------------------------------

async function isUp(url) {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(2000) });
    return res.ok;
  } catch {
    return false;
  }
}

async function ensureDevServer() {
  if (await isUp(APP_URL)) {
    log('dev server already running at', APP_URL);
    return null;
  }
  log('starting react-router dev server…');
  const child = spawn('npx', ['react-router', 'dev'], {
    cwd: __dirname,
    stdio: 'ignore',
    detached: false,
  });
  for (let i = 0; i < 60; i++) {
    if (await isUp(APP_URL)) return child;
    await sleep(500);
  }
  child.kill();
  throw new Error('dev server did not come up on ' + APP_URL);
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

let failures = 0;
function assert(cond, label) {
  if (cond) {
    log('PASS:', label);
  } else {
    failures += 1;
    console.error('[check] FAIL:', label);
  }
}

async function main() {
  await resetTargetItem();
  await cleanupConversations();
  const cost0 = await costSummary();

  const dev = await ensureDevServer();
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    page.on('console', (msg) => {
      if (msg.type() === 'error') console.error('[browser]', msg.text());
    });
    page.on('pageerror', (err) => console.error('[browser] PAGEERROR:', err.message));

    // 1. Login ---------------------------------------------------------------
    await page.goto(APP_URL, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.login-card, [data-testid="workspace"]', { timeout: 15000 });
    if (await page.locator('.login-card').count()) {
      await page.click('.login-card button[type="submit"]'); // prefilled creds
    }
    await page.waitForSelector('[data-testid="workspace"]', { timeout: 20000 });
    assert(true, 'login → workspace (useAuth + RaisinProvider)');

    // 2. Checklist + connection ----------------------------------------------
    await page.waitForSelector('[data-testid="item-row"]', { timeout: 20000 });
    const itemCount = await page.locator('[data-testid="item-row"]').count();
    assert(itemCount === 5, `checklist renders 5 items (useSql) — got ${itemCount}`);
    await page.waitForSelector('[data-testid="conn-dot"].connected', { timeout: 10000 });
    assert(true, 'connection dot green (useConnection)');

    const targetBefore = await page
      .locator(`[data-testid="item-row"][data-path="${TARGET_ITEM}"]`)
      .getAttribute('data-status');
    assert(targetBefore === 'todo', `target item starts as todo (got ${targetBefore})`);

    const modeChip = await page.locator('[data-testid="mode-chip"]').getAttribute('data-mode');
    assert(modeChip === 'approve_then_auto', `mode chip shows agent execution_mode (${modeChip})`);

    // 3. Ask for the plan (ONE Groq run starts here) -------------------------
    const input = page.locator('[data-testid="chat-input"]');
    await input.fill(CHAT_PROMPT);
    await input.press('Enter');
    await page.waitForSelector('[data-testid="bubble-user"]', { timeout: 10000 });
    assert(true, 'user message bubble appears');

    // 4. Plan proposal card with Approve (pending_approval) ------------------
    const proposal = page.locator('[data-testid="plan-card"][data-status="pending_approval"]');
    await proposal.waitFor({ timeout: 120000 });
    assert(true, 'plan card appears with status pending_approval');
    await proposal.locator('[data-testid="plan-approve"]').waitFor({ timeout: 10000 });
    assert(true, 'Approve/Reject buttons rendered (requiresApproval)');
    const taskCount = await proposal.locator('[data-testid="task-row"]').count();
    assert(taskCount >= 2, `plan proposes ≥ 2 ordered tasks (got ${taskCount})`);
    await page.screenshot({ path: '/tmp/shots/taskpilot-plan-proposed.png', fullPage: true });

    // 5. Approve → tasks execute to completion --------------------------------
    await proposal.locator('[data-testid="plan-approve"]').click();
    log('plan approved — waiting for execution…');

    await page.waitForSelector('[data-testid="plan-card"][data-status="completed"]', {
      timeout: 240000,
    });
    assert(true, 'plan card progressed to completed');
    const doneTasks = await page
      .locator('[data-testid="task-row"][data-status="completed"]')
      .count();
    assert(doneTasks >= 2, `all plan tasks completed in the UI (got ${doneTasks})`);

    // 6. The checklist item actually changed ----------------------------------
    const rows = await adminSql(`SELECT properties FROM 'projects' WHERE path = $1`, [
      TARGET_ITEM,
    ]);
    assert(rows[0]?.properties?.status === 'done', 'item status=done on the server');

    // …and the live UI shows it (subscription/refetch)
    try {
      await page.waitForSelector(
        `[data-testid="item-row"][data-path="${TARGET_ITEM}"][data-status="done"]`,
        { timeout: 30000 },
      );
      assert(true, 'checklist UI shows the item as done (live update)');
    } catch {
      assert(false, 'checklist UI shows the item as done (live update)');
    }

    // 7. Final reply summarizes ----------------------------------------------
    // Plan/task lifecycle messages are filtered out of the transcript, so
    // the last assistant bubble must be the model's actual closing summary.
    let replyText = '';
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
      const bubbles = page.locator('[data-testid="bubble-assistant"]');
      const n = await bubbles.count();
      if (n > 0) {
        replyText = (await bubbles.last().innerText()).trim();
        if (/done|summar|checklist|complete/i.test(replyText)) break;
      }
      await sleep(1000);
    }
    assert(
      replyText.length > 0 && /done|summar|checklist|complete/i.test(replyText),
      `assistant summary reply received: "${replyText.slice(0, 120)}"`,
    );

    await page.screenshot({ path: '/tmp/shots/taskpilot-plan-completed.png', fullPage: true });
    log('screenshots: /tmp/shots/taskpilot-plan-{proposed,completed}.png');
  } finally {
    await browser.close();
    if (dev) dev.kill();
  }

  const cost1 = await costSummary();
  log(`Groq spend: ${cost1.calls - cost0.calls} LLM calls, ${cost1.tokens - cost0.tokens} tokens`);
  assert(cost1.calls - cost0.calls <= 10, 'run used ≤ 10 LLM calls');

  if (failures > 0) {
    console.error(`[check] ${failures} assertion(s) failed`);
    process.exit(1);
  }
  log('ALL CHECKS PASSED');
}

main().catch((err) => {
  console.error('[check] ERROR:', err);
  process.exit(1);
});
