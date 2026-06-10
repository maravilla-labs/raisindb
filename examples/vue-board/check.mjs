#!/usr/bin/env node
/**
 * Headless end-to-end check for the vue-board demo.
 *
 * Flow: login (prefilled demo creds) → board renders shifts + staff →
 * connection dot green → one cheap chat turn against the shift-planner agent
 * (read-only list tool; board state is NOT changed) → assert a streamed
 * reply → trigger a harmless node:updated (rewrite a shift's properties
 * with identical values via the admin HTTP API) and assert the live
 * highlight fires on the matching card.
 *
 * Prereqs:
 *   - raisin-server on RAISIN_URL (default http://localhost:8081) with the
 *     shiftboard package installed into repo RAISIN_REPO (default shiftboard2)
 *   - Playwright installed somewhere; default lookup dir /tmp/shots/node_modules
 *     (override with PLAYWRIGHT_DIR)
 *   - If no dev server is already on APP_URL, this script starts `vite` itself.
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
const REPO = process.env.RAISIN_REPO ?? 'shiftboard2';
const TENANT = process.env.RAISIN_TENANT ?? 'default';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const APP_URL = process.env.APP_URL ?? 'http://localhost:5176';
const PLAYWRIGHT_DIR = process.env.PLAYWRIGHT_DIR ?? '/tmp/shots/node_modules';
const CHAT_PROMPT = 'Which shifts are open this weekend?';

const require = createRequire(path.join(PLAYWRIGHT_DIR, 'noop.js'));
const { chromium } = require('playwright');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...args) => console.log('[check]', ...args);

// ---------------------------------------------------------------------------
// Server-side helpers (plain HTTP, mirrors examples/shiftboard/setup.mjs)
// ---------------------------------------------------------------------------

async function plannerSql(query) {
  const login = await fetch(`${BASE_URL}/auth/${REPO}/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: 'planner@example.com', password: 'Planner12345!' }),
  });
  if (!login.ok) throw new Error(`planner login failed: HTTP ${login.status}`);
  const { access_token } = await login.json();
  const res = await fetch(`${BASE_URL}/api/sql/${REPO}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${access_token}`,
    },
    body: JSON.stringify({ sql: query }),
  });
  if (!res.ok) throw new Error(`sql failed: HTTP ${res.status} ${await res.text()}`);
  return (await res.json()).rows ?? [];
}

/**
 * Rewrite the given properties (identical values) on a shift node via the
 * admin SDK over WebSocket → the server emits node:updated to subscribers.
 *
 * The update response can hang on the current dev server (post-commit
 * stall) even though the write applies, so the request is fire-and-forget
 * with a short timeout.
 */
async function touchShift(shiftPath, properties) {
  const { RaisinClient } = await import('@raisindb/client');
  const wsUrl = process.env.VITE_RAISIN_WS_URL ?? `ws://localhost:8081/ws/${REPO}`;
  const client = new RaisinClient(wsUrl, { requestTimeout: 10000 });
  await client.connect();
  await client.authenticate({ username: ADMIN_USER, password: ADMIN_PASSWORD });
  const db = client.database(REPO);
  const rows = (
    await db.executeSql(`SELECT id FROM 'staffing' WHERE path = '${shiftPath}'`)
  ).rows;
  if (!rows?.length) throw new Error(`node not found: ${shiftPath}`);
  try {
    await db.workspace('staffing').nodes().update(rows[0].id, { properties });
  } catch (err) {
    log('touch request did not return (write still applies):', err.message);
  } finally {
    client.disconnect();
  }
}

// ---------------------------------------------------------------------------
// Dev server (reuse a running one, else start vite ourselves)
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
  log('starting vite dev server…');
  const child = spawn('npx', ['vite', '--port', '5176', '--strictPort'], {
    cwd: __dirname,
    stdio: 'ignore',
    detached: false,
  });
  for (let i = 0; i < 60; i++) {
    if (await isUp(APP_URL)) return child;
    await sleep(500);
  }
  child.kill();
  throw new Error('vite dev server did not come up on ' + APP_URL);
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
  const vite = await ensureDevServer();
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    page.on('console', (msg) => {
      if (msg.type() === 'error') console.error('[browser]', msg.text());
    });

    // 1. Login -------------------------------------------------------------
    await page.goto(APP_URL, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.login-card, [data-testid="board"]', { timeout: 15000 });

    if (await page.locator('.login-card').count()) {
      // Demo creds are prefilled — just submit.
      await page.click('.login-card button[type="submit"]');
    }

    // 2. Board -------------------------------------------------------------
    await page.waitForSelector('[data-testid="shift-card"]', { timeout: 20000 });
    const shiftCount = await page.locator('[data-testid="shift-card"]').count();
    assert(shiftCount >= 5, `board renders shifts (${shiftCount} cards)`);

    const staffCount = await page.locator('[data-testid="staff-list"] .staff-row').count();
    assert(staffCount >= 3, `staff list renders (${staffCount} rows)`);

    // 3. Connection dot ----------------------------------------------------
    await page.waitForSelector('[data-testid="conn-dot"].connected', { timeout: 10000 });
    assert(true, 'connection dot is green (useConnection)');

    // 4. Live board update (harmless: rewrite the shift's own title) -------
    //
    // KNOWN SERVER ISSUE (2026-06-10, local dev server): client-initiated
    // full-node updates (WS NodeUpdate / HTTP PUT) commit the write but the
    // request hangs post-commit (before event emission), so node:updated
    // never reaches subscribers. SKIP_TOUCH=1 skips this step; without it
    // the step times out and is reported as a failure.
    if (process.env.SKIP_TOUCH === '1') {
      log('SKIP_TOUCH=1 — skipping live-update assertion');
    } else {
      const firstCard = page.locator('[data-testid="shift-card"]').first();
      const shiftPath = await firstCard.getAttribute('data-path');
      log('touching shift node', shiftPath);

      const rows = await plannerSql(
        `SELECT path, properties FROM 'staffing' WHERE path = '${shiftPath}'`,
      );
      if (rows.length !== 1) throw new Error(`expected 1 row for ${shiftPath}`);
      const originalProps = rows[0].properties;

      // Rewrite ONLY the title with its identical value (merge semantics
      // leave every other property untouched; avoids the WS null->'' trap
      // on `assignee`).
      await touchShift(shiftPath, { title: originalProps.title });

      try {
        await page.waitForSelector(
          `[data-testid="shift-card"][data-path="${shiftPath}"].flash`,
          { timeout: 20000 },
        );
        assert(true, 'live node:updated event flashed the card (useSubscription)');
      } catch {
        assert(false, 'live node:updated event flashed the card (useSubscription)');
      }

      // Verify the board state is unchanged on the server.
      const after = await plannerSql(
        `SELECT path, properties FROM 'staffing' WHERE path = '${shiftPath}'`,
      );
      const canon = (p) =>
        JSON.stringify(Object.keys(p).sort().map((k) => [k, p[k]]));
      assert(
        canon(after[0]?.properties ?? {}) === canon(originalProps),
        'shift properties unchanged after touch',
      );
    }

    // 5. Chat: one cheap read-only turn -------------------------------------
    const input = page.locator('[data-testid="chat-input"]');
    await input.waitFor({ timeout: 15000 });
    const bubblesBefore = await page.locator('[data-testid="bubble-assistant"]').count();

    await input.fill(CHAT_PROMPT);
    await input.press('Enter'); // Enter-to-send

    await page.waitForSelector(`[data-testid="bubble-user"]:has-text("weekend")`, {
      timeout: 10000,
    });
    assert(true, 'user message bubble appears');

    // Streaming text and/or tool badges show up while the agent works; the
    // turn ends with a persisted assistant bubble.
    let sawStreaming = false;
    let sawToolBadge = false;
    const deadline = Date.now() + 90000;
    let replyText = '';
    while (Date.now() < deadline) {
      if (!sawStreaming && (await page.locator('[data-testid="streaming"]').count())) {
        sawStreaming = true;
      }
      if (!sawToolBadge && (await page.locator('[data-testid="tool-badge"]').count())) {
        sawToolBadge = true;
      }
      const bubbles = page.locator('[data-testid="bubble-assistant"]');
      if ((await bubbles.count()) > bubblesBefore) {
        replyText = (await bubbles.last().innerText()).trim();
        if (replyText) break;
      }
      await sleep(250);
    }
    const isProviderError = /failed to get ai config|configuration not found/i.test(replyText);
    if (isProviderError) {
      // The chat pipeline (send → SSE stream → bubble) works, but the server
      // has no AI provider key configured, so the model call itself failed.
      log(
        'WARN: chat pipeline verified end-to-end, but the server has no AI',
        'provider configured — reply is the backend error, not a model reply:',
        replyText.slice(0, 100),
      );
      assert(replyText.length > 0, 'chat round-trip delivered a (error) reply via SSE');
    } else {
      assert(
        replyText.length > 0 && !/^Error:/.test(replyText),
        `assistant reply received (${replyText.slice(0, 80)}…)`,
      );
      assert(sawStreaming || sawToolBadge, 'reply was streamed (streaming bubble or tool badge seen)');
    }
    log('saw streaming bubble:', sawStreaming, '| saw tool badge:', sawToolBadge);

    // 6. Bell counter (soft check — the agent reply lands in the inbox) ----
    const badge = await page.locator('[data-testid="bell-badge"]').count();
    log('bell badge visible:', badge > 0 ? await page.locator('[data-testid="bell-badge"]').innerText() : 'no');

    await page.screenshot({ path: '/tmp/shots/vue-board-final.png', fullPage: true });
    log('screenshot: /tmp/shots/vue-board-final.png');
  } finally {
    await browser.close();
    if (vite) vite.kill();
  }

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
