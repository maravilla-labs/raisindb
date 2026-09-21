/**
 * The finalize gate: free text cannot override a stopped or failed state, and an
 * agent cannot complete its own build task without evidence.
 *
 * Moved here from a workstream scratchpad, unchanged apart from the path, so
 * it runs with the rest of ai-tools. The evidence-ROUTING cases (derived
 * targets, sibling evidence, staleness) are in evidence-routing.test.mjs.
 *
 * Run: node --test builtin-packages/ai-tools/tests/finalize-gate.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const AI = join(dirname(fileURLToPath(import.meta.url)), '../content/functions/lib/raisin/ai');

/** A minimal raisin host: a node store plus a SQL fake that pattern-matches. */
function host(nodes) {
  const store = new Map(Object.entries(nodes));
  globalThis.raisin = {
    nodes: {
      async get(ws, path) {
        const n = store.get(path);
        return n ? { id: path, path, ...n } : null;
      },
      async update(ws, path, patch) {
        const n = store.get(path) || {};
        store.set(path, { ...n, ...patch });
        return { path };
      },
      async create(ws, parent, spec) {
        const path = `${parent}/${spec.name}`;
        store.set(path, { node_type: spec.node_type, properties: spec.properties });
        return { path };
      },
    },
    sql: {
      async query(sql, params) {
        const rows = [...store.entries()].map(([path, n]) => ({ path, id: path, ...n }));
        const isTask = (n) => n.node_type === 'raisin:AITask';
        if (/id = \$2/.test(sql)) {
          return rows.filter((r) => isTask(r) && r.path.startsWith(params[0]) && r.path === params[1])
            .map((r) => ({ id: r.path, path: r.path, properties: r.properties }));
        }
        if (/COUNT\(\*\) as total/.test(sql)) {
          return [{ total: rows.filter((r) => isTask(r) && parentOf(r.path) === params[0]).length }];
        }
        if (/COUNT\(\*\) as completed/.test(sql)) {
          assert.ok(!/::String/.test(sql), 'no String cast in the predicate');
          return [{ completed: rows.filter((r) => isTask(r) && parentOf(r.path) === params[0] && r.properties.status === 'completed').length }];
        }
        if (/node_type = 'raisin:AITask'/.test(sql) && /CHILD_OF/.test(sql)) {
          return rows.filter((r) => isTask(r) && parentOf(r.path) === params[0]).map((r) => ({ path: r.path, properties: r.properties }));
        }
        if (/node_type = 'raisin:AITask'/.test(sql) && /DESCENDANT_OF/.test(sql)) {
          return rows.filter((r) => isTask(r) && r.path.startsWith(params[0])).map((r) => ({ path: r.path, properties: r.properties }));
        }
        if (/raisin:Message/.test(sql)) return [];
        return [];
      },
    },
  };
  return store;
}
const parentOf = (p) => p.split('/').slice(0, -1).join('/');

const CHAT = '/chats/c1';
const PLAN = '/chats/c1/plan';
const TASK = '/chats/c1/plan/t1';

/** A conversation whose agent declares (or does not declare) the gate. */
function fixture({ policy, taskProps }) {
  return host({
    [CHAT]: { node_type: 'raisin:Chat', properties: { agent_ref: { 'raisin:path': '/agents/builder', 'raisin:workspace': 'functions' } } },
    '/agents/builder': { node_type: 'raisin:AIAgent', properties: policy ? { finalize_policy: policy } : {} },
    [PLAN]: { node_type: 'raisin:AIPlan', properties: { status: 'in_progress' } },
    [TASK]: { node_type: 'raisin:AITask', properties: { title: 'Build the title automation', status: 'in_progress', ...taskProps } },
  });
}

const call = async (status = 'completed') => {
  const { handler } = await import(`${AI}/update-task/index.js`);
  return handler({ task_id: TASK, status, __raisin_context: { workspace: 'ai', chat_path: CHAT } });
};

test('a build task with no verification record cannot be completed', async () => {
  const store = fixture({ policy: 'require_verified_completion', taskProps: { build_target_path: '/automations/correct-page-title-on-update' } });
  const res = await call();
  assert.equal(res.success, false);
  assert.equal(res.reason, 'unverified');
  assert.ok(res.missing.includes('verification_ref'), res.missing.join('|'));
  assert.equal(store.get(TASK).properties.status, 'in_progress', 'status must be unchanged');
});

test('an agent without the policy still completes its tasks', async () => {
  const store = fixture({ policy: null, taskProps: { build_target_path: '/automations/correct-page-title-on-update' } });
  const res = await call();
  assert.equal(res.success, true);
  assert.equal(res.new_status, 'completed');
  assert.equal(store.get(TASK).properties.status, 'completed');
});

test('a gated task with a complete record completes, and the plan follows', async () => {
  const target = '/automations/correct-page-title-on-update';
  const store = fixture({
    policy: 'require_verified_completion',
    taskProps: { build_target_path: target, verification_ref: { 'raisin:path': target }, verification_hash: 'rev-7', proof_level: 'draft_validated' },
  });
  const res = await call();
  assert.equal(res.success, true);
  assert.equal(store.get(TASK).properties.status, 'completed');
  assert.equal(store.get(PLAN).properties.status, 'completed');
});

test('a plan of self-ticked tasks does not complete without evidence', async () => {
  const { updatePlanProgress, FINALIZE_POLICY_VERIFIED } = await import(`${AI}/update-task/index.js`);
  const store = fixture({ policy: 'require_verified_completion', taskProps: { build_target_path: '/automations/x', status: 'completed' } });
  const progress = await updatePlanProgress('ai', PLAN, CHAT, FINALIZE_POLICY_VERIFIED);
  assert.equal(progress.total_tasks, 1);
  assert.equal(progress.completed_tasks, 1);
  assert.equal(progress.status, 'in_progress', 'plan must not auto-complete');
  assert.equal(progress.unverified_tasks.length, 1);
  assert.equal(store.get(PLAN).properties.status, 'in_progress');
});

test('the same plan, ungated, completes exactly as before', async () => {
  const { updatePlanProgress } = await import(`${AI}/update-task/index.js`);
  fixture({ policy: null, taskProps: { build_target_path: '/automations/x', status: 'completed' } });
  const progress = await updatePlanProgress('ai', PLAN, CHAT, 'none');
  assert.equal(progress.status, 'completed');
});

// ── the terminal-response path ───────────────────────────────────────────────

test('a terminal turn with an open task states STOPPED and makes no readiness claim', async () => {
  const { gateTerminalContent } = await import(`${AI}/agent-handler/index.js`);
  fixture({ policy: 'require_verified_completion', taskProps: {} });
  const out = await gateTerminalContent('ai', CHAT, { finalize_policy: 'require_verified_completion' },
    'All done! The automation is validated and enabled and ready to use.');
  assert.equal(out.gated, true);
  assert.match(out.content, /STOPPED with 1 task\(s\) still open/);
  // A POSITIVE readiness claim, not the server's own negation of one.
  assert.doesNotMatch(out.content, /\b(is|are) (validated and )?enabled\b|ready to use/i);
});

test('a terminal turn with an unverified completed build says UNVERIFIED', async () => {
  const { gateTerminalContent } = await import(`${AI}/agent-handler/index.js`);
  fixture({ policy: 'require_verified_completion', taskProps: { status: 'completed', build_target_path: '/automations/x' } });
  const out = await gateTerminalContent('ai', CHAT, { finalize_policy: 'require_verified_completion' }, 'Ready and enabled.');
  assert.equal(out.gated, true);
  assert.match(out.content, /UNVERIFIED/);
  assert.doesNotMatch(out.content, /\bReady and enabled\b/i);
  assert.doesNotMatch(out.content, /(?<!not )\benabled\b/i);
});

test('draft validation says execution untested, and keeps the agent prose', async () => {
  const { gateTerminalContent } = await import(`${AI}/agent-handler/index.js`);
  const target = '/automations/x';
  fixture({ policy: 'require_verified_completion', taskProps: { status: 'completed', build_target_path: target, verification_ref: target, verification_hash: 'h', proof_level: 'draft_validated' } });
  const out = await gateTerminalContent('ai', CHAT, { finalize_policy: 'require_verified_completion' }, 'I built the automation.');
  assert.equal(out.gated, false);
  assert.match(out.content, /draft validated; execution untested/);
  assert.match(out.content, /I built the automation\./);
});

test('an agent without the policy has its content passed through untouched', async () => {
  const { gateTerminalContent } = await import(`${AI}/agent-handler/index.js`);
  fixture({ policy: null, taskProps: {} });
  const out = await gateTerminalContent('ai', CHAT, {}, 'Ready and enabled.');
  assert.equal(out.content, 'Ready and enabled.');
  assert.equal(out.statement, null);
});

test('a stopped run leaves no task in in_progress', async () => {
  const { failOpenTasks } = await import(`${AI}/agent-continue-handler/index.js`);
  const store = fixture({ policy: 'require_verified_completion', taskProps: {} });
  store.set('/chats/c1/plan/t2', { node_type: 'raisin:AITask', properties: { title: 'Ticked itself', status: 'completed' } });
  const closed = await failOpenTasks('ai', CHAT, 'max_continuation_depth');
  assert.equal(closed.length, 1);
  assert.equal(store.get(TASK).properties.status, 'failed');
  assert.equal(store.get('/chats/c1/plan/t2').properties.status, 'completed', 'a terminal task is left alone');
  for (const [, n] of store) {
    if (n.node_type === 'raisin:AITask') assert.notEqual(n.properties.status, 'in_progress');
  }
});

// ── added by the adversarial review: a check that could not run is not a pass ──

test('an unreadable task query is UNVERIFIED, not a pass', async () => {
  const { gateTerminalContent } = await import(`${AI}/agent-handler/index.js`);
  fixture({ policy: 'require_verified_completion', taskProps: { status: 'completed' } });
  // The one failure the gate must not wave through: the query itself throws.
  globalThis.raisin.sql.query = async () => { throw new Error('index unavailable'); };
  const out = await gateTerminalContent('ai', CHAT, { finalize_policy: 'require_verified_completion' },
    'All done! The automation is validated and enabled and ready to use.');
  assert.equal(out.gated, true);
  assert.match(out.content, /UNVERIFIED/);
  assert.doesNotMatch(out.content, /ready to use/i);
});
