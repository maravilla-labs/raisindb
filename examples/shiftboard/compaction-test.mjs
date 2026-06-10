#!/usr/bin/env node
/**
 * Shiftboard demo - token safeguards & auto-compaction e2e test.
 *
 * Proves the agent token safeguards end to end against a live dev server:
 *   1. Redeploys the agent-handler / agent-shared function code from the
 *      builtin-packages sources (repeatable; inline `code` property shadows
 *      the packaged binary asset).
 *   2. Reconfigures /agents/shift-planner to a cheap model with
 *      auto_compact=true, compact_threshold_messages=6,
 *      max_conversation_tokens=4000.
 *   3. Sends 4 short user turns establishing facts; after crossing the
 *      threshold asserts a raisin:AICompaction node exists under the
 *      agent-side conversation, then asserts the next reply still knows an
 *      EARLY (compacted) fact - proving the summary carries context.
 *   4. Drops the budget to 100 tokens, sends one more message, asserts a
 *      budget-exceeded reply with finish_reason=budget_exceeded and that NO
 *      new AI call happened (cost-record count unchanged).
 *   5. Restores the agent's original configuration.
 *
 * Costs real Groq tokens (~6 short calls on llama-3.1-8b-instant).
 *
 * Run: node compaction-test.mjs
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { RaisinClient, MemoryTokenStorage } from '@raisindb/client';

const HTTP_URL = process.env.RAISIN_HTTP_URL ?? 'http://localhost:8081';
const WS_URL = process.env.RAISIN_WS_URL ?? 'ws://localhost:8081/ws/shiftboard2';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard2';
const ADMIN_USER = process.env.RAISIN_USER ?? 'admin';
const ADMIN_PASSWORD = process.env.RAISIN_PASSWORD ?? 'Admin12345!@#';
const PLANNER_EMAIL = 'planner@example.com';
const PLANNER_PASSWORD = 'Planner12345!';
const AGENT_PATH = '/agents/shift-planner';

const __dirname = dirname(fileURLToPath(import.meta.url));
const AI_FUNCS_SRC = join(__dirname, '../../builtin-packages/ai-tools/content/functions/lib/raisin/ai');

/** Function source files (relative to lib/raisin/ai) to deploy to the live repo. */
const DEPLOY_FILES = [
  'agent-handler/index.js',
  'agent-continue-handler/index.js',
  'agent-shared/history.js',
  'agent-shared/utils.js',
  'agent-shared/compaction.js',
  'agent-shared/index.js',
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
}

// ── Admin HTTP helpers ────────────────────────────────────────────────────────

let adminToken = null;

async function api(method, path, body) {
  const res = await fetch(`${HTTP_URL}${path}`, {
    method,
    headers: {
      ...(adminToken ? { Authorization: `Bearer ${adminToken}` } : {}),
      ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  let json = null;
  try { json = text ? JSON.parse(text) : null; } catch { /* non-JSON */ }
  return { status: res.status, json, text };
}

async function adminLogin() {
  const { status, json } = await api('POST', '/api/raisindb/sys/default/auth', {
    username: ADMIN_USER,
    password: ADMIN_PASSWORD,
  });
  assert(status === 200 && json?.token, `admin login failed (${status})`);
  adminToken = json.token;
}

const nodeUrl = (ws, path) => `/api/repository/${REPO}/main/head/${ws}${path}`;

async function getNode(ws, path) {
  const { status, json } = await api('GET', nodeUrl(ws, path));
  if (status === 404) return null;
  assert(status === 200, `GET ${path} -> ${status}`);
  return json?.node ?? json;
}

async function putNodeProperties(ws, path, properties) {
  const { status, text } = await api('PUT', nodeUrl(ws, path), { properties });
  assert(status >= 200 && status < 300, `PUT ${path} -> ${status}: ${text.slice(0, 200)}`);
}

// ── Step A: workspace + code deployment ──────────────────────────────────────

async function ensureCompactionAllowedInAiWorkspace() {
  const { status, json: ws } = await api('GET', `/api/workspaces/${REPO}/ai`);
  assert(status === 200 && ws, `GET ai workspace -> ${status}`);
  if (!ws.allowed_node_types.includes('raisin:AICompaction')) {
    ws.allowed_node_types.push('raisin:AICompaction');
    const put = await api('PUT', `/api/workspaces/${REPO}/ai`, ws);
    assert(put.status >= 200 && put.status < 300, `PUT ai workspace -> ${put.status}`);
    console.log('✅ ai workspace: allowed raisin:AICompaction');
  } else {
    console.log('✅ ai workspace already allows raisin:AICompaction');
  }
}

async function deployFunctionSources() {
  for (const rel of DEPLOY_FILES) {
    const code = await readFile(join(AI_FUNCS_SRC, rel), 'utf8');
    const assetPath = `/lib/raisin/ai/${rel}`;
    const existing = await getNode('functions', assetPath);
    if (existing) {
      await putNodeProperties('functions', assetPath, {
        ...(existing.properties ?? {}),
        code,
      });
    } else {
      // New module (e.g. agent-shared/compaction.js) — create the asset node
      const parts = assetPath.split('/');
      const name = parts.pop();
      const parent = parts.join('/');
      // raisin:Asset requires a `file` Resource; the function code loader
      // prefers the inline `code` property, so a synthetic descriptor is fine.
      const now = new Date().toISOString();
      const { status, text } = await api('POST', nodeUrl('functions', parent), {
        name,
        node_type: 'raisin:Asset',
        properties: {
          title: name,
          file_type: 'text/javascript',
          code,
          file: {
            uuid: `inline-${Date.now()}`,
            name,
            size: code.length,
            mime_type: 'text/javascript',
            created_at: now,
            updated_at: now,
          },
        },
      });
      assert(status >= 200 && status < 300, `POST ${assetPath} -> ${status}: ${text.slice(0, 200)}`);
    }
    // Verify the deployment took effect
    const check = await getNode('functions', assetPath);
    assert(check?.properties?.code === code, `deployed code mismatch for ${assetPath}`);
    console.log(`✅ deployed ${assetPath} (${code.length} bytes)`);
  }
}

// ── Step B: agent reconfiguration ─────────────────────────────────────────────

async function configureAgent(overrides, removeKeys = []) {
  const agent = await getNode('functions', AGENT_PATH);
  assert(agent, `agent not found: ${AGENT_PATH}`);
  const props = { ...(agent.properties ?? {}), ...overrides };
  for (const k of removeKeys) delete props[k];
  await putNodeProperties('functions', AGENT_PATH, props);
  return agent.properties ?? {};
}

// ── Chat helpers (SDK, identity user) ─────────────────────────────────────────

async function streamTurn(db, conversationPath, text) {
  console.log(`\n>>> USER: ${text}`);
  const seen = { done: null, failed: null, text: '' };
  const timeout = AbortSignal.timeout(120_000);
  for await (const event of db.conversations.sendMessage(conversationPath, text, {
    stream: true,
    signal: timeout,
  })) {
    if (event.type === 'text_chunk') {
      seen.text += event.text ?? '';
      process.stdout.write(event.text ?? '');
    } else if (event.type === 'done') {
      seen.done = event;
    } else if (event.type === 'failed') {
      seen.failed = event;
    }
    if (seen.done || seen.failed) break;
  }
  console.log();
  if (seen.failed) throw new Error(`Turn failed: ${JSON.stringify(seen.failed)}`);
  if (!seen.text && seen.done?.content) seen.text = seen.done.content;
  return seen;
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  console.log(`Compaction & token-budget test against ${HTTP_URL} (repo ${REPO})\n`);
  await adminLogin();
  console.log('✅ admin logged in');

  await ensureCompactionAllowedInAiWorkspace();
  await deployFunctionSources();

  // Reconfigure agent: cheap model + compaction + budget (keep tools etc.)
  const originalProps = await configureAgent({
    model: 'llama-3.1-8b-instant',
    max_tokens: 256,
    auto_compact: true,
    compact_threshold_messages: 6,
    max_conversation_tokens: 4000,
  });
  console.log('✅ agent configured: llama-3.1-8b-instant, auto_compact on, threshold 6, budget 4000');

  // Admin SQL client (ai workspace is RLS-protected)
  const adminClient = new RaisinClient(WS_URL, {
    tokenStorage: new MemoryTokenStorage(),
    tenantId: 'default',
  });
  await adminClient.connect();
  await adminClient.authenticate({ username: ADMIN_USER, password: ADMIN_PASSWORD });
  const adminDb = adminClient.database(REPO);

  let client = null;
  try {
    // Chat as identity user
    client = new RaisinClient(WS_URL, {
      tokenStorage: new MemoryTokenStorage(),
      tenantId: 'default',
      requestTimeout: 30000,
    });
    const user = await client.loginWithEmail(PLANNER_EMAIL, PLANNER_PASSWORD, REPO);
    console.log(`✅ logged in as ${user.email}`);
    const db = client.database(REPO);

    const convo = await db.conversations.create({
      participant: AGENT_PATH,
      subject: 'Compaction test',
    });
    console.log(`✅ conversation: ${convo.conversationPath}`);
    const agentConv = `/agents/shift-planner/inbox/chats/${convo.conversationPath.split('/').pop()}`;

    // 4 short fact turns. Threshold 6 means compaction triggers at the start
    // of turn 4 (7 direct messages exist when its handler runs).
    const facts = ['apple', 'banana', 'cherry', 'plum'];
    for (let i = 0; i < facts.length; i++) {
      await streamTurn(
        db,
        convo.conversationPath,
        `Please remember this for later and confirm in one short sentence (no tools): fact #${i + 1} = '${facts[i]}'.`,
      );
    }

    // Assert a compaction node exists under the agent-side conversation
    await sleep(1500);
    const compactions = await adminDb.executeSql(
      `SELECT path, properties FROM 'ai' WHERE CHILD_OF($1) AND node_type = 'raisin:AICompaction'`,
      [agentConv],
    );
    assert((compactions.rows?.length ?? 0) > 0, 'a raisin:AICompaction node must exist after crossing the threshold');
    const comp = compactions.rows[compactions.rows.length - 1];
    console.log('\n✅ AICompaction node:', comp.path);
    console.log(JSON.stringify({
      messages_compacted: comp.properties?.messages_compacted,
      messages_kept: comp.properties?.messages_kept,
      cutoff_message_path: comp.properties?.cutoff_message_path,
      summary_preview: comp.properties?.summary_preview,
    }, null, 2));
    assert(comp.properties?.summary, 'compaction node must store the full summary');
    assert(comp.properties?.cutoff_message_path, 'compaction node must record the cutoff message path');

    // The summarization call must be accounted for
    const compCost = await adminDb.executeSql(
      `SELECT path, properties FROM 'ai' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
      [comp.path],
    );
    assert((compCost.rows?.length ?? 0) === 1, 'summarization call must have a cost record');
    console.log(`✅ summarization cost record: ${compCost.rows[0].properties?.total_tokens} tokens`);

    // Recall an EARLY fact — it lives only in the compacted summary now
    const recall = await streamTurn(
      db,
      convo.conversationPath,
      "What was fact #1? Answer with just the word.",
    );
    assert(
      recall.text.toLowerCase().includes('apple'),
      `agent must recall compacted fact #1 'apple' via the summary, got: ${recall.text}`,
    );
    console.log("✅ agent recalled compacted fact #1 ('apple') from the summary");

    // ── Budget test ──
    const convNode = await adminDb.executeSql(
      `SELECT path, properties FROM 'ai' WHERE path = $1`,
      [agentConv],
    );
    const used = convNode.rows?.[0]?.properties?.total_tokens_used ?? 0;
    console.log(`\nℹ️ conversation total_tokens_used = ${used}`);
    assert(used > 100, 'running total must exceed the test budget of 100 by now');

    await configureAgent({ max_conversation_tokens: 100 });
    console.log('✅ agent budget dropped to 100 tokens');

    const costsBefore = await adminDb.executeSql(
      `SELECT path FROM 'ai' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
      [agentConv],
    );
    const nCostsBefore = costsBefore.rows?.length ?? 0;

    const budgetTurn = await streamTurn(db, convo.conversationPath, 'And what was fact #2?');
    assert(
      budgetTurn.text.includes('reached its token budget'),
      `expected budget-exceeded reply, got: ${budgetTurn.text}`,
    );
    assert(
      budgetTurn.done?.finishReason === 'budget_exceeded',
      `expected finishReason budget_exceeded, got: ${budgetTurn.done?.finishReason}`,
    );
    console.log(`✅ budget refusal: "${budgetTurn.text}" (finishReason=${budgetTurn.done?.finishReason})`);

    await sleep(1000);
    const costsAfter = await adminDb.executeSql(
      `SELECT path FROM 'ai' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
      [agentConv],
    );
    assert(
      (costsAfter.rows?.length ?? 0) === nCostsBefore,
      `no new cost record may exist after a budget refusal (before=${nCostsBefore}, after=${costsAfter.rows?.length})`,
    );
    console.log(`✅ no Groq call happened for the refused turn (${nCostsBefore} cost records unchanged)`);

    console.log('\n🎉 Compaction & token-budget test PASSED.');
  } finally {
    // Restore the demo agent exactly as it was
    try {
      await putNodeProperties('functions', AGENT_PATH, originalProps);
      console.log('✅ agent restored to original configuration');
    } catch (e) {
      console.error('⚠️ failed to restore agent config:', e.message);
    }
    await adminClient.disconnect?.();
    await client?.disconnect?.();
  }
  process.exit(0);
}

main().catch((err) => {
  console.error('\n❌ Compaction test FAILED:', err.message || err);
  process.exit(1);
});
