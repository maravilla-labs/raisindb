/**
 * EVIDENCE ROUTING for the finalize gate (baseline gap 8, 2026-09-21).
 *
 * Replays the shapes measured on the dev server in Studio Builder's round 2:
 *   - B: a separate "Test" task refused `unverified` five times while the
 *     record for the same function sat on its sibling "Draft" task;
 *   - A: a refused completion that named `build_target_path` did not keep it,
 *     and a later call that simply left it out closed the task `success:true`
 *     right after the task had created an automation;
 *   - a re-draft after the test left the old record standing.
 *
 * The host is a node store plus a SQL fake that understands the four queries
 * the gate makes: the task by id, the conversation's tasks, its tool calls and
 * their results. Tool calls carry `created_at` like the real rows do.
 *
 * Run: node --test builtin-packages/ai-tools/tests/evidence-routing.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const AI = join(dirname(fileURLToPath(import.meta.url)), '../content/functions/lib/raisin/ai');
const { handler, updatePlanProgress, FINALIZE_POLICY_VERIFIED } = await import(`${AI}/update-task/index.js`);
const runEvidence = await import(`${AI}/agent-shared/run-evidence.js`);
const { gateTerminalContent } = await import(`${AI}/agent-shared/finalize.js`);

const CHAT = '/agents/builder/inbox/chats/c1';
const PLAN = `${CHAT}/reply-1/plan-1`;
const FN = '/lib/studio/generated/enforce-homepage-title';
const AUTO = '/enforce-homepage-title';
const parentOf = (p) => p.split('/').slice(0, -1).join('/');

/** A minimal raisin host. `store` maps path → node; nodes carry created_at. */
function host(nodes, { failToolQuery = false } = {}) {
  const store = new Map(Object.entries(nodes));
  const updates = [];
  globalThis.raisin = {
    nodes: {
      async get(ws, path) {
        const n = store.get(path);
        return n ? { id: n.id || path, path, ...n } : null;
      },
      async update(ws, path, patch) {
        updates.push({ path, patch });
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
          assert.ok(!/::String/.test(sql), 'no String cast in the predicate');
          if (failToolQuery) throw new Error('index unavailable');
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
  return { store, updates };
}

/** The conversation, its agent and plan, and whatever tasks/calls a test adds. */
function conversation({ policy = FINALIZE_POLICY_VERIFIED, tasks = {}, calls = [] } = {}) {
  const nodes = {
    [CHAT]: { node_type: 'raisin:Chat', properties: { agent_ref: { 'raisin:path': '/agents/builder', 'raisin:workspace': 'functions' } } },
    '/agents/builder': { node_type: 'raisin:AIAgent', properties: policy ? { finalize_policy: policy } : {} },
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
    if (c.result !== undefined) {
      nodes[`${path}/result`] = {
        node_type: 'raisin:AIToolSingleCallResult',
        created_at: c.done || c.at,
        properties: { function_name: c.tool, tool_call_id: `tc-${i}`, result: c.result },
      };
    }
  });
  return host(nodes);
}

const complete = (task_id, extra = {}) =>
  handler({ task_id, status: 'completed', ...extra, __raisin_context: { workspace: 'ai', chat_path: CHAT } });

const T = (s) => `2026-09-21T10:${s}+00:00`;
const fnEvidence = (at, version = 3) => ({
  verification_ref: { 'raisin:ref': 'fn-id', 'raisin:workspace': 'functions', 'raisin:path': FN },
  verification_hash: `source_version:${version}`,
  proof_level: 'fixture_tested',
  verified_at: at,
});
const started = (task_id, at) => ({ tool: 'update-task', at, args: { task_id, status: 'in_progress' }, result: { success: true } });
const drafted = (at, task) => ({
  tool: 'draft-function', at, args: { path: FN },
  result: { success: true, path: FN, source_path: `${FN}/index.js`, created: false, ...(task ? { task: { task_id: task } } : {}) },
});

// ── B: evidence for the artifact counts wherever it sits ─────────────────────

test('B: a Test task closes on the Draft task\'s record for the same function', async () => {
  const { store } = conversation({
    tasks: {
      draft: { status: 'completed', build_target_path: FN, ...fnEvidence(T('08:17.000')) },
      test: {},
    },
    calls: [drafted(T('08:05.000'), 'draft'), started('test', T('08:12.000'))],
  });
  const res = await complete('test', { build_target_path: FN });
  assert.equal(res.success, true, JSON.stringify(res));
  assert.equal(res.evidence_from_task, `${PLAN}/draft`);
  const props = store.get(`${PLAN}/test`).properties;
  assert.equal(props.status, 'completed');
  assert.equal(props.verification_ref['raisin:path'], FN);
  assert.equal(props.proof_level, 'fixture_tested');
  assert.equal(store.get(PLAN).properties.status, 'completed', 'both tasks verified, so the plan completes');
});

test('B: the terminal statement agrees with the adopted record', async () => {
  conversation({
    tasks: {
      draft: { status: 'completed', build_target_path: FN, ...fnEvidence(T('08:17.000')) },
      test: {},
    },
    calls: [drafted(T('08:05.000'), 'draft'), started('test', T('08:12.000'))],
  });
  await complete('test', { build_target_path: FN });
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, 'Done.');
  assert.equal(out.gated, false, out.content);
  assert.match(out.content, /fixture-tested/);
});

// ── A: a refused completion keeps its declaration ────────────────────────────

test('A: a refused completion persists build_target_path', async () => {
  const { store } = conversation({ tasks: { draft: {} }, calls: [started('draft', T('09:24.000'))] });
  const res = await complete('draft', { build_target_path: FN });
  assert.equal(res.success, false);
  assert.equal(res.reason, 'unverified');
  assert.equal(res.build_target_persisted, FN);
  const props = store.get(`${PLAN}/draft`).properties;
  assert.equal(props.build_target_path, FN, 'the declaration survives the refusal');
  assert.equal(props.status, 'in_progress', 'the status does not');
});

// ── A: not declaring is not an escape ────────────────────────────────────────

test('A: a task that created an automation is a build task without declaring one', async () => {
  const { store } = conversation({
    tasks: { test: { build_target_path: undefined } },
    calls: [
      started('test', T('49:47.000')),
      { tool: 'propose-automation', at: T('49:53.000'), args: {}, result: { success: true, deployable: false } },
      { tool: 'create-node', at: T('50:00.000'), args: { workspace: 'automations', name: 'enforce-homepage-title' },
        result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' } },
    ],
  });
  const res = await complete('test');
  assert.equal(res.success, false, 'the 09:50:14 bypass is refused');
  assert.equal(res.reason, 'unverified');
  assert.deepEqual(res.derived_from_writes.map((w) => [w.artifact, w.tool]), [['automations:/enforce-homepage-title', 'create-node']]);
  assert.equal(store.get(`${PLAN}/test`).properties.build_target_path, AUTO, 'the derived target is persisted for the next verification');
  assert.equal(store.get(`${PLAN}/test`).properties.status, 'in_progress');
});

test('A: once verify-automation stamps a task naming the automation, the derived task closes', async () => {
  const { store } = conversation({
    tasks: {
      create: {},
      verify: {
        status: 'completed',
        build_target_path: `/automations${AUTO}`,
        verification_ref: { 'raisin:ref': AUTO, 'raisin:workspace': 'automations', 'raisin:path': `/automations${AUTO}` },
        verification_hash: 'sha256:abc',
        proof_level: 'draft_validated',
        verified_at: T('51:00.000'),
      },
    },
    calls: [
      started('create', T('50:00.000')),
      { tool: 'create-node', at: T('50:01.000'), args: {}, result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' } },
    ],
  });
  const res = await complete('create');
  assert.equal(res.success, true, JSON.stringify(res));
  const props = store.get(`${PLAN}/create`).properties;
  assert.equal(props.build_target_path, AUTO);
  assert.equal(props.verification_ref['raisin:path'], AUTO, 'the envelope is spelled the way this task spells its target');
});

test('a task that wrote nothing still closes on the agent\'s word', async () => {
  const { store } = conversation({
    tasks: { discover: {} },
    calls: [
      started('discover', T('28:48.000')),
      { tool: 'discover-capabilities', at: T('28:49.000'), args: { query: 'x' }, result: { success: true, results: [] } },
      { tool: 'execute-function', at: T('28:50.000'), args: { path: FN }, result: { success: true, path: FN, was_enabled: false } },
      { tool: 'update-node', at: T('28:51.000'), args: { path: FN }, result: { success: true, path: FN, changed: [] } },
      { tool: 'update-node', at: T('28:52.000'), args: { path: FN }, result: { success: false, reason: 'arming_is_human', path: FN, changed: [] } },
      { tool: 'materialize-automation', at: T('28:53.000'), args: { automation_path: AUTO }, result: { success: true, flow_path: '/flows/x', unchanged: true } },
      { tool: 'materialize-automation', at: T('28:54.000'), args: { automation_path: AUTO }, result: { success: true, dry_run: true, flow_path: '/flows/x' } },
    ],
  });
  const res = await complete('discover');
  assert.equal(res.success, true, JSON.stringify(res));
  assert.equal(store.get(`${PLAN}/discover`).properties.build_target_path, undefined);
});

test('a write a tool bound to ANOTHER task is not this task\'s', async () => {
  const res = await (async () => {
    conversation({
      tasks: { request: {}, draft: {} },
      calls: [started('request', T('30:00.000')), drafted(T('30:01.000'), 'draft')],
    });
    return complete('request');
  })();
  assert.equal(res.success, true, JSON.stringify(res));
});

test('B 10:12:46: a write made while ANOTHER task was current is that task\'s, not the earlier one\'s', async () => {
  // Draft started 10:08:03; the agent then moved to "Request arming" (10:08:24)
  // and created the automation there (10:08:57). Draft's completion is about
  // its function only.
  const { store } = conversation({
    tasks: { draft: { build_target_path: FN, ...fnEvidence(T('12:11.000'), 4) }, arming: {} },
    calls: [
      started('draft', T('08:03.000')),
      started('arming', T('08:24.000')),
      { tool: 'create-node', at: T('08:57.000'), args: {}, result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' } },
      drafted(T('12:07.000'), 'draft'),
    ],
  });
  const res = await complete('draft');
  assert.equal(res.success, true, JSON.stringify(res));
  const arming = await complete('arming');
  assert.equal(arming.success, false, 'the automation it created is charged to the task that was current');
  assert.equal(store.get(`${PLAN}/arming`).properties.build_target_path, AUTO);
});

test('completing straight from pending does not dodge a write made while no task was open', async () => {
  conversation({
    tasks: { quick: {} },
    calls: [
      { tool: 'create-node', at: T('20:00.000'), args: {}, result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' } },
    ],
  });
  const res = await complete('quick');
  assert.equal(res.success, false);
});

// ── staleness ───────────────────────────────────────────────────────────────

test('a re-draft after the test makes the task\'s own record stale', async () => {
  const { store } = conversation({
    tasks: { draft: { build_target_path: FN, ...fnEvidence(T('08:17.000')) } },
    calls: [
      started('draft', T('08:00.000')),
      drafted(T('08:05.000'), 'draft'),
      drafted(T('12:00.000'), 'draft'),
    ],
  });
  const res = await complete('draft');
  assert.equal(res.success, false);
  assert.ok(res.stale && res.stale.some((s) => /stale: draft-function wrote/.test(s)), JSON.stringify(res));
  assert.equal(store.get(`${PLAN}/draft`).properties.status, 'in_progress');
});

test('a re-test after the re-draft makes it fresh again', async () => {
  const res = await (async () => {
    conversation({
      tasks: { draft: { build_target_path: FN, ...fnEvidence(T('12:11.000'), 4) } },
      calls: [started('draft', T('08:00.000')), drafted(T('08:05.000'), 'draft'), drafted(T('12:00.000'), 'draft')],
    });
    return complete('draft');
  })();
  assert.equal(res.success, true, JSON.stringify(res));
});

test('a sibling\'s stale record is not adopted', async () => {
  conversation({
    tasks: {
      draft: { status: 'completed', build_target_path: FN, ...fnEvidence(T('08:17.000')) },
      test: {},
    },
    calls: [drafted(T('08:05.000'), 'draft'), started('test', T('08:12.000')), drafted(T('09:00.000'), 'draft')],
  });
  const res = await complete('test', { build_target_path: FN });
  assert.equal(res.success, false);
  assert.ok(res.stale.length >= 1);
});

test('a completed task whose artifact was re-written keeps the plan open and the turn UNVERIFIED', async () => {
  const { store } = conversation({
    tasks: { draft: { status: 'completed', build_target_path: FN, ...fnEvidence(T('08:17.000')) } },
    calls: [drafted(T('08:05.000'), 'draft'), drafted(T('09:00.000'), 'draft')],
  });
  const progress = await updatePlanProgress('ai', PLAN, CHAT, FINALIZE_POLICY_VERIFIED);
  assert.equal(progress.status, 'in_progress');
  assert.equal(progress.unverified_tasks.length, 1);
  assert.equal(store.get(PLAN).properties.status, 'in_progress');

  const before = await gateTerminalContent('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, 'Ready.');
  assert.equal(before.gated, false, 'finalize.js alone cannot see the re-draft');
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, 'Ready.');
  assert.equal(out.gated, true);
  assert.match(out.content, /UNVERIFIED/);
});

// ── failure and non-gated paths ─────────────────────────────────────────────

test('an unreadable tool-call record refuses the completion rather than passing it', async () => {
  const { store } = host({
    [CHAT]: { node_type: 'raisin:Chat', properties: { agent_ref: { 'raisin:path': '/agents/builder', 'raisin:workspace': 'functions' } } },
    '/agents/builder': { node_type: 'raisin:AIAgent', properties: { finalize_policy: FINALIZE_POLICY_VERIFIED } },
    [PLAN]: { node_type: 'raisin:AIPlan', properties: { status: 'in_progress' } },
    [`${PLAN}/t`]: { id: 't', node_type: 'raisin:AITask', properties: { title: 't', status: 'in_progress' } },
  }, { failToolQuery: true });
  const res = await complete('t');
  assert.equal(res.success, false);
  assert.match(res.message, /could not be read/);
  assert.equal(store.get(`${PLAN}/t`).properties.status, 'in_progress');
});

test('an agent without the policy is untouched by derived targets', async () => {
  const { store } = conversation({
    policy: null,
    tasks: { create: {} },
    calls: [
      started('create', T('50:00.000')),
      { tool: 'create-node', at: T('50:01.000'), args: {}, result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' } },
    ],
  });
  const res = await complete('create');
  assert.equal(res.success, true);
  assert.equal(store.get(`${PLAN}/create`).properties.build_target_path, undefined);
});

// ── the pure pieces ─────────────────────────────────────────────────────────

test('writesOfResult: the explicit contract, the current shapes, and the non-writes', () => {
  const { writesOfResult } = runEvidence;
  assert.deepEqual(writesOfResult({ success: true, artifact: { workspace: 'w', path: '/a' } }, {}), [{ workspace: 'w', path: '/a' }]);
  assert.deepEqual(writesOfResult({ success: true, path: '/a', created: true }, {}), [{ workspace: null, path: '/a' }]);
  assert.deepEqual(writesOfResult({ success: true, path: '/a', changed: ['title'] }, { workspace: 'stories' }), [{ workspace: 'stories', path: '/a' }]);
  assert.deepEqual(writesOfResult({ success: true, flow_path: '/f' }, { automation_path: '/x' }), [{ workspace: 'automations', path: '/x' }]);
  assert.deepEqual(writesOfResult({ success: false, path: '/a', created: true }, {}), []);
  assert.deepEqual(writesOfResult({ success: true, path: '/a', changed: [] }, {}), []);
  assert.deepEqual(writesOfResult({ success: true, path: '/a' }, {}), [], 'a read that echoes a path is not a write');
  assert.deepEqual(writesOfResult(JSON.stringify({ success: true, path: '/a', created: false }), {}), [{ workspace: null, path: '/a' }]);
});

test('sameArtifact: bare, workspace-prefixed and enveloped spellings of one node', () => {
  const { sameArtifact, artifactOf } = runEvidence;
  assert.ok(sameArtifact(artifactOf(null, '/automations/x'), artifactOf('automations', '/x')));
  assert.ok(sameArtifact(artifactOf('automations', '/automations/x'), artifactOf(null, '/x')));
  assert.ok(sameArtifact(artifactOf(null, FN), artifactOf('functions', FN)));
  assert.ok(!sameArtifact(artifactOf('stories', '/x'), artifactOf('automations', '/x')));
  assert.ok(!sameArtifact(artifactOf(null, '/x'), artifactOf(null, '/y')));
});

// ── adversarial review (2026-09-21): ways round the derived-target gate ──────

const createdAutoAt = (at) => ({
  tool: 'create-node', at, args: { workspace: 'automations', name: 'enforce-homepage-title' },
  result: { success: true, workspace: 'automations', path: AUTO, id: 'a1', node_type: 'studio:Automation' },
});

test('review: a write made before any task was started belongs to the task picked up next', async () => {
  conversation({
    tasks: { build: { status: 'pending' } },
    calls: [createdAutoAt(T('10:00.000')), started('build', T('10:05.000'))],
  });
  const res = await complete('build');
  assert.equal(res.success, false, 'create first, start the task afterwards, is not a way round the gate');
  assert.equal(res.reason, 'unverified');
});

test('review: an unowned write cannot be laundered through another task started after it', async () => {
  conversation({
    tasks: { other: { status: 'pending' }, build: { status: 'pending' } },
    calls: [createdAutoAt(T('10:00.000')), started('other', T('10:05.000'))],
  });
  const res = await complete('other');
  assert.equal(res.success, false, 'the task picked up after the write owns it');
});

test('review: a completion batched beside a still-running write is refused until it answers', async () => {
  const { store } = conversation({
    tasks: { t: {} },
    calls: [started('t', T('10:00.000')), { tool: 'create-node', at: T('10:01.000'), args: { workspace: 'automations' } }],
  });
  const p = `${CHAT}/turn-1/tool-call-1`;
  store.set(p, { ...store.get(p), properties: { ...store.get(p).properties, status: 'running' } });
  const res = await handler({ task_id: 't', status: 'completed', __raisin_context: { workspace: 'ai', chat_path: CHAT, msg_path: `${CHAT}/turn-1` } });
  assert.equal(res.success, false);
  assert.deepEqual(res.in_flight, ['create-node']);
  assert.equal(store.get(`${PLAN}/t`).properties.status, 'in_progress');
});

test('review: the terminal statement covers writes no completed task owns (no plan, or a cancelled writer)', async () => {
  conversation({ tasks: {}, calls: [createdAutoAt(T('10:00.000'))] });
  const bare = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, 'Built and enabled.');
  assert.equal(bare.gated, true);
  assert.match(bare.content, /UNVERIFIED — no verification record for .*automations:\/enforce-homepage-title/);

  conversation({
    tasks: { a: { status: 'cancelled' } },
    calls: [started('a', T('10:00.000')), createdAutoAt(T('10:01.000')), { tool: 'update-task', at: T('10:02.000'), args: { task_id: 'a', status: 'cancelled' }, result: { success: true } }],
  });
  const cancelled = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, 'Built and enabled.');
  assert.equal(cancelled.gated, true);
  assert.doesNotMatch(cancelled.content, /Built and enabled/);
});

test('review: a verified write does not trip the terminal statement', async () => {
  conversation({
    tasks: {
      v: {
        status: 'completed', build_target_path: AUTO,
        verification_ref: { 'raisin:ref': AUTO, 'raisin:workspace': 'automations', 'raisin:path': AUTO },
        verification_hash: 'sha256:x', proof_level: 'draft_validated', verified_at: T('10:05.000'),
      },
    },
    calls: [createdAutoAt(T('10:00.000'))],
  });
  const out = await runEvidence.gateTerminalContentWithRunWrites('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, 'Done.');
  assert.equal(out.gated, false, out.content);
});

test('review: materializing a deleted automation is not a build target', () => {
  assert.deepEqual(runEvidence.writesOfResult({ success: true, deleted: true, disarmed: [], flow_path: '/f' }, { automation_path: '/gone' }), []);
});

test('review: the malformed-call stop text reports an unverified write, like the terminal gate', async () => {
  const { malformedToolCallStopText } = await import(`${AI}/agent-shared/completion-retry.js`);
  conversation({ tasks: {}, calls: [createdAutoAt(T('10:00.000'))] });
  const text = await malformedToolCallStopText('ai', CHAT, { finalize_policy: FINALIZE_POLICY_VERIFIED }, null);
  assert.match(text, /This turn stopped/);
  assert.match(text, /UNVERIFIED — no verification record for .*automations:\/enforce-homepage-title/);
});
