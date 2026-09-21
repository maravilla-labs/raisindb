/**
 * SUPPORTING ARTIFACTS for the finalize gate (review, 2026-09-21).
 *
 * No tool writes a verification record for an AGENT, so a task that called
 * `upsert-agent` — the natural way to build an AI decider — could never be
 * completed. An agent is verified THROUGH a ready `verify-automation` of an
 * automation that calls it (that check resolves the agent, type-checks it and
 * reads its output schema). These tests pin both halves: an agent built for a
 * verified automation closes, and an agent nothing verified still refuses.
 *
 * The host is the same node store + SQL fake as evidence-routing.test.mjs.
 *
 * Run: node --test builtin-packages/ai-tools/tests/supporting-artifacts.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const AI = join(dirname(fileURLToPath(import.meta.url)), '../content/functions/lib/raisin/ai');
const { handler, updatePlanProgress, FINALIZE_POLICY_VERIFIED } = await import(`${AI}/update-task/index.js`);
const runEvidence = await import(`${AI}/agent-shared/run-evidence.js`);

const CHAT = '/agents/builder/inbox/chats/c1';
const PLAN = `${CHAT}/reply-1/plan-1`;
const AGENT = '/agents/order-decider';
const AUTO = '/route-orders';
const parentOf = (p) => p.split('/').slice(0, -1).join('/');
const POLICY = { finalize_policy: FINALIZE_POLICY_VERIFIED };

function host(nodes) {
  const store = new Map(Object.entries(nodes));
  globalThis.raisin = {
    nodes: {
      async get(ws, path) {
        const n = store.get(path);
        return n ? { id: n.id || path, path, ...n } : null;
      },
      async update(ws, path, patch) {
        const n = store.get(path) || {};
        store.set(path, { ...n, ...patch });
        return { path };
      },
    },
    sql: {
      async query(sql, params) {
        const rows = [...store.entries()].map(([path, n]) => ({ path, id: n.id || path, ...n }));
        const under = (r) => r.path.startsWith(`${params[0]}/`);
        const typed = (t) => rows.filter((r) => r.node_type === t && under(r));
        if (/raisin:AIToolCall'/.test(sql) || /raisin:AIToolSingleCallResult'/.test(sql)) {
          const t = /AIToolCall'/.test(sql) ? 'raisin:AIToolCall' : 'raisin:AIToolSingleCallResult';
          return typed(t).map((r) => ({ path: r.path, properties: r.properties, created_at: r.created_at }));
        }
        if (/id = \$2/.test(sql)) {
          return typed('raisin:AITask').filter((r) => r.id === params[1])
            .map((r) => ({ id: r.id, path: r.path, properties: r.properties }));
        }
        if (/raisin:AITask'/.test(sql) && /CHILD_OF/.test(sql)) {
          return rows.filter((r) => r.node_type === 'raisin:AITask' && parentOf(r.path) === params[0])
            .map((r) => ({ path: r.path, properties: r.properties }));
        }
        if (/raisin:AITask'/.test(sql) && /DESCENDANT_OF/.test(sql)) {
          return typed('raisin:AITask').map((r) => ({ id: r.id, path: r.path, properties: r.properties }));
        }
        return [];
      },
    },
  };
  return { store };
}

function conversation({ tasks = {}, calls = [] } = {}) {
  const nodes = {
    [CHAT]: { node_type: 'raisin:Chat', properties: { agent_ref: { 'raisin:path': '/agents/builder', 'raisin:workspace': 'functions' } } },
    '/agents/builder': { node_type: 'raisin:AIAgent', properties: POLICY },
    [PLAN]: { node_type: 'raisin:AIPlan', properties: { status: 'in_progress' } },
  };
  for (const [name, props] of Object.entries(tasks)) {
    nodes[`${PLAN}/${name}`] = { id: name, node_type: 'raisin:AITask', properties: { title: name, status: 'in_progress', ...props } };
  }
  calls.forEach((c, i) => {
    const path = `${CHAT}/turn-${i}/tool-call-${i}`;
    nodes[path] = {
      node_type: 'raisin:AIToolCall',
      created_at: c.at,
      properties: { function_name: c.tool, tool_call_id: `tc-${i}`, arguments: c.args || {}, status: 'completed' },
    };
    nodes[`${path}/result`] = {
      node_type: 'raisin:AIToolSingleCallResult',
      created_at: c.done || c.at,
      properties: { function_name: c.tool, tool_call_id: `tc-${i}`, result: c.result },
    };
  });
  return host(nodes);
}

const complete = (task_id, extra = {}) =>
  handler({ task_id, status: 'completed', ...extra, __raisin_context: { workspace: 'ai', chat_path: CHAT } });

const T = (s) => `2026-09-21T11:${s}+00:00`;
const started = (task_id, at) => ({ tool: 'update-task', at, args: { task_id, status: 'in_progress' }, result: { success: true } });
const upsertAgent = (at) => ({
  tool: 'upsert-agent', at, args: { slug: 'order-decider', title: 'Order decider' },
  result: { success: true, path: AGENT, created: true },
});
const createAuto = (at) => ({
  tool: 'create-node', at, args: { workspace: 'automations', name: 'route-orders' },
  result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' },
});
const verifyAuto = (at, { ready = true, deps = [{ path: AGENT, workspace: 'functions', kind: 'agent', id: 'ag1', enabled: true }], dryRun = false } = {}) => ({
  tool: 'verify-automation', at, args: { automation_path: AUTO, ...(dryRun ? { dry_run: true } : {}) },
  result: {
    success: true, automation_path: AUTO, ready, level: ready ? 'draft_validated' : null,
    verification: { authored_hash: 'sha256:auto', level: ready ? 'draft_validated' : null, dependencies: deps, ready },
  },
});
/** What verify-automation stamps on the task that names the automation. */
const autoRecord = (at) => ({
  build_target_path: AUTO,
  verification_ref: { 'raisin:ref': AUTO, 'raisin:workspace': 'automations', 'raisin:path': AUTO },
  verification_hash: 'sha256:auto',
  proof_level: 'draft_validated',
  verified_at: at,
});

// ── an agent built for a verified automation completes ───────────────────────

test('a build that creates an agent AND an automation, with the automation verified, completes', async () => {
  const { store } = conversation({
    tasks: { build: autoRecord(T('03:00.500')) },
    calls: [started('build', T('01:00.000')), upsertAgent(T('01:10.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000'))],
  });
  const res = await complete('build');
  assert.equal(res.success, true, JSON.stringify(res));
  const props = store.get(`${PLAN}/build`).properties;
  assert.equal(props.status, 'completed');
  assert.equal(props.build_target_path, AUTO, 'the automation, not the agent, is what the record is routed by');

  const progress = await updatePlanProgress('ai', PLAN, CHAT, FINALIZE_POLICY_VERIFIED);
  assert.equal(progress.status, 'completed');
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, POLICY, 'Ready.');
  assert.equal(out.gated, false, out.content);
  assert.doesNotMatch(out.content, /UNVERIFIED/);
});

test('the same build, before the automation was verified, is refused and names the automation as its target', async () => {
  const { store } = conversation({
    tasks: { build: {} },
    calls: [started('build', T('01:00.000')), upsertAgent(T('01:10.000')), createAuto(T('02:00.000'))],
  });
  const res = await complete('build');
  assert.equal(res.success, false);
  assert.equal(res.build_target, AUTO, 'the directly verifiable artifact is primary even though the agent was written first');
  assert.equal(store.get(`${PLAN}/build`).properties.build_target_path, AUTO);
});

test('a task that DECLARED the agent as its target is re-routed to the automation it also wrote', async () => {
  const { store } = conversation({
    tasks: { build: { build_target_path: AGENT } },
    calls: [started('build', T('01:00.000')), upsertAgent(T('01:10.000')), createAuto(T('02:00.000'))],
  });
  const res = await complete('build');
  assert.equal(res.success, false);
  assert.equal(store.get(`${PLAN}/build`).properties.build_target_path, AUTO,
    'otherwise verify-automation would stamp nobody and the task could never close');
});

test('an agent-only task closes on a sibling automation\'s verification that depends on it', async () => {
  const { store } = conversation({
    tasks: { agent: {}, automation: { status: 'completed', ...autoRecord(T('03:00.500')) } },
    calls: [
      started('agent', T('01:00.000')), upsertAgent(T('01:10.000')),
      started('automation', T('01:30.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000')),
    ],
  });
  const res = await complete('agent');
  assert.equal(res.success, true, JSON.stringify(res));
  assert.equal(res.evidence_from_task, `${PLAN}/automation`);
  const props = store.get(`${PLAN}/agent`).properties;
  assert.equal(props.build_target_path, AGENT);
  assert.equal(props.verification_ref['raisin:path'], AGENT, 'the record is about the AGENT, so a later agent write makes it stale');
  assert.equal(props.verification_ref['raisin:workspace'], 'functions');
  assert.equal(props.proof_level, 'draft_validated', 'no stronger than the automation\'s own proof');
  assert.equal(props.verification_via, 'automations:/route-orders');

  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, POLICY, 'Ready.');
  assert.equal(out.gated, false, out.content);
});

// ── an agent nothing verified still refuses ──────────────────────────────────

test('an agent-only task with no verified dependent is refused, and says how to verify it', async () => {
  const { store } = conversation({
    tasks: { agent: {} },
    calls: [started('agent', T('01:00.000')), upsertAgent(T('01:10.000'))],
  });
  const res = await complete('agent');
  assert.equal(res.success, false);
  assert.equal(res.reason, 'unverified');
  assert.match(res.message, /agent has no verifier of its own/);
  assert.match(res.message, /verify-automation on an automation whose agent step calls it/);
  assert.equal(store.get(`${PLAN}/agent`).properties.status, 'in_progress');

  // Giving up on the task does not launder the write: the turn still says so.
  const cancelled = await handler({ task_id: 'agent', status: 'cancelled', __raisin_context: { workspace: 'ai', chat_path: CHAT } });
  assert.equal(cancelled.success, true);
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, POLICY, 'Ready.');
  assert.equal(out.gated, true);
  assert.match(out.content, /UNVERIFIED — no verification record for "written by upsert-agent" → \/agents\/order-decider/);
});

test('a verification made BEFORE the agent was re-written does not cover it', async () => {
  conversation({
    tasks: { agent: {}, automation: { status: 'completed', ...autoRecord(T('03:00.500')) } },
    calls: [
      started('agent', T('01:00.000')), upsertAgent(T('01:10.000')),
      started('automation', T('01:30.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000')),
      started('agent', T('04:00.000')), upsertAgent(T('04:10.000')),
    ],
  });
  const res = await complete('agent');
  assert.equal(res.success, false, JSON.stringify(res));
  assert.ok(res.stale.some((s) => /predates a later write/.test(s)), JSON.stringify(res.stale));
});

test('a verification that is not ready, a dry run, or does not list the agent covers nothing', async () => {
  for (const verify of [
    verifyAuto(T('03:00.000'), { ready: false }),
    verifyAuto(T('03:00.000'), { dryRun: true }),
    verifyAuto(T('03:00.000'), { deps: [{ path: '/agents/someone-else', workspace: 'functions', kind: 'agent' }] }),
  ]) {
    conversation({
      tasks: { agent: {}, automation: { status: 'completed', ...autoRecord(T('03:00.500')) } },
      calls: [started('agent', T('01:00.000')), upsertAgent(T('01:10.000')), createAuto(T('02:00.000')), verify],
    });
    const res = await complete('agent');
    assert.equal(res.success, false, JSON.stringify(verify.args));
  }
});

test('a ready verification whose automation holds no current record covers nothing', async () => {
  conversation({
    tasks: { agent: {} },
    calls: [started('agent', T('01:00.000')), upsertAgent(T('01:10.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000'))],
  });
  const res = await complete('agent');
  assert.equal(res.success, false);
  assert.ok(res.stale.some((s) => /no task holds a current record/.test(s)), JSON.stringify(res.stale));
});

test('an adopted agent record goes stale when the agent is written again', async () => {
  const { store } = conversation({
    tasks: { agent: {}, automation: { status: 'completed', ...autoRecord(T('03:00.500')) } },
    calls: [
      started('agent', T('01:00.000')), upsertAgent(T('01:10.000')),
      started('automation', T('01:30.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000')),
    ],
  });
  assert.equal((await complete('agent')).success, true);
  // The agent is re-written after its task closed.
  store.set(`${CHAT}/turn-9/tool-call-9`, { node_type: 'raisin:AIToolCall', created_at: T('05:00.000'),
    properties: { function_name: 'upsert-agent', tool_call_id: 'tc-9', arguments: {}, status: 'completed' } });
  store.set(`${CHAT}/turn-9/tool-call-9/result`, { node_type: 'raisin:AIToolSingleCallResult', created_at: T('05:00.000'),
    properties: { function_name: 'upsert-agent', tool_call_id: 'tc-9', result: { success: true, path: AGENT, created: false } } });
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, POLICY, 'Ready.');
  assert.equal(out.gated, true);
  assert.match(out.content, /UNVERIFIED/);
});

test('supportingKind: agents are supporting, automations and functions are not', () => {
  const { supportingKind, artifactOf } = runEvidence;
  assert.equal(supportingKind(artifactOf(null, AGENT)), 'agent');
  assert.equal(supportingKind(artifactOf(null, `/functions${AGENT}`)), 'agent');
  assert.equal(supportingKind(artifactOf('functions', AGENT)), 'agent');
  assert.equal(supportingKind(artifactOf('automations', AGENT)), null, 'a node elsewhere named agents is not an agent');
  assert.equal(supportingKind(artifactOf(null, CHAT)), null, 'a conversation below an agent is not the agent');
  assert.equal(supportingKind(artifactOf('automations', AUTO)), null);
  assert.equal(supportingKind(artifactOf(null, '/lib/studio/generated/x')), null);
});

// ── adversarial review: the agent's coverage must not leak (2026-09-21) ──────

test('an AUTOMATION stored at automations:/agents/x does not ride the coverage of the agent /agents/x', async () => {
  // upsert-agent reports no workspace, so its write and this automation's spell
  // the same path; the automation was never verified.
  const shadow = { tool: 'create-node', at: T('04:00.000'), args: { workspace: 'automations', name: 'order-decider' },
    result: { success: true, workspace: 'automations', path: AGENT, id: 'shadow', node_type: 'studio:Automation' } };
  conversation({
    tasks: { shadow: { build_target_path: AGENT }, automation: { status: 'completed', ...autoRecord(T('03:00.500')) } },
    calls: [
      upsertAgent(T('01:10.000')), started('automation', T('01:30.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000')),
      started('shadow', T('03:30.000')), shadow, verifyAuto(T('05:00.000')),
    ],
  });
  const res = await complete('shadow');
  assert.equal(res.success, false, JSON.stringify(res));

  const cancelled = await handler({ task_id: 'shadow', status: 'cancelled', __raisin_context: { workspace: 'ai', chat_path: CHAT } });
  assert.equal(cancelled.success, true);
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, POLICY, 'Ready.');
  assert.equal(out.gated, true, out.content);
  assert.match(out.content, /UNVERIFIED — no verification record for "written by create-node" → automations:\/agents\/order-decider/);
});

test('an agent write that cannot be dated is never shown to predate a verification', async () => {
  conversation({
    tasks: { agent: {}, automation: { status: 'completed', ...autoRecord(T('03:00.500')) } },
    calls: [
      started('agent', T('01:00.000')), upsertAgent(T('01:10.000')),
      started('automation', T('01:30.000')), createAuto(T('02:00.000')), verifyAuto(T('03:00.000')),
      upsertAgent('not a time'),
    ],
  });
  const res = await complete('agent');
  assert.equal(res.success, false, JSON.stringify(res));
});
