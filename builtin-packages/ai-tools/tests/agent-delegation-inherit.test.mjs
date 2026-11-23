/**
 * A delegated child of a CONFIGURED agent behaves like that agent at top
 * level: its execution mode (a person still approves where the agent says
 * so), its `run_budgets` and `on_exceeded`, and its `node_dev.roots` as the
 * write grant — narrowed by the parent's grant and the requested `writes`,
 * never widened. And what a child asks for (an approval, a question) is
 * visible to the parent in inspect-agent and wait-for-agents.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-delegation-inherit.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { childWorld } from './support/fake-child-runs.mjs';
import {
  childBudgets, intersectRoots, childGrant, agentRoots, openRequestsOf, childBlocked,
} from '../content/functions/lib/raisin/ai/agent-shared/delegation-spec.js';
import { handler as spawn } from '../content/functions/lib/raisin/ai/spawn-agent/index.js';
import { handler as inspectAgent } from '../content/functions/lib/raisin/ai/inspect-agent/index.js';
import { handler as waitAgents } from '../content/functions/lib/raisin/ai/wait-for-agents/index.js';

const FN = {
  spawn: '/lib/raisin/ai/spawn-agent',
  inspect: '/lib/raisin/ai/inspect-agent',
  wait: '/lib/raisin/ai/wait-for-agents',
};

const BUILDER_OWN = { max_model_calls: 40, max_operations: 240, max_consecutive_op_failures: 6, max_total_tokens: 300000, on_exceeded: 'pause' };
const AUTOMATIONS = { workspace: 'automations', path: '/', ops: ['read', 'create', 'update', 'delete'] };
const TAGS = { workspace: 'tags', path: '/', ops: ['read'] };

// ── Pure: budgets ───────────────────────────────────────────────────────────

test('budgets: an agent\'s own run_budgets are the defaults; a request lowers, never raises; the ceiling holds', () => {
  const b = childBudgets(null, BUILDER_OWN);
  assert.equal(b.max_model_calls, 40);
  assert.equal(b.max_operations, 240);
  assert.equal(b.max_total_tokens, 300000);
  assert.equal(b.max_consecutive_op_failures, 6);
  assert.equal(b.max_wall_ms, 60 * 60 * 1000, 'an undeclared limit is the top-level default');
  assert.equal(b.on_exceeded, 'pause', 'on_exceeded as configured');

  const lowered = childBudgets({ max_model_calls: 10, max_operations: 1000, on_exceeded: 'pause' }, BUILDER_OWN);
  assert.equal(lowered.max_model_calls, 10);
  assert.equal(lowered.max_operations, 240, 'a request cannot exceed the agent\'s own');
  assert.equal(lowered.on_exceeded, 'pause');
  assert.equal(childBudgets({ on_exceeded: 'fail' }, BUILDER_OWN).on_exceeded, 'fail', 'a parent may ask for fail');
  assert.equal(childBudgets({ on_exceeded: 'pause' }, { max_model_calls: 5 }).on_exceeded, 'fail', 'pause only when the agent says so');

  const huge = childBudgets(null, { max_model_calls: 10000, max_total_tokens: 1e12 });
  assert.equal(huge.max_model_calls, 60, 'the engine ceiling');
  assert.equal(huge.max_total_tokens, 2000000);

  const plain = childBudgets({ max_model_calls: 50 });
  assert.equal(plain.max_model_calls, 50, 'without run_budgets the old rule: request clamped to the ceiling');
  assert.equal(plain.max_operations, 80);
  assert.equal(plain.on_exceeded, 'fail');
});

// ── Pure: grant ─────────────────────────────────────────────────────────────

test('grant: the agent\'s roots, narrowed by the parent and by writes, never widened; nothing left is refused', () => {
  const own = agentRoots({ node_dev: { roots: [AUTOMATIONS, TAGS, { path: '/x' }] } });
  assert.deepEqual(own, [AUTOMATIONS, TAGS], 'a root without a workspace is dropped');
  assert.equal(agentRoots({}), null);

  // The parent's grant narrows paths and ops; a pair with no common op goes.
  const narrowed = intersectRoots(own, [{ workspace: 'automations', path: '/flows', ops: ['read', 'update', 'move'] }]);
  assert.deepEqual(narrowed, [{ workspace: 'automations', path: '/flows', ops: ['read', 'update'] }]);
  assert.deepEqual(intersectRoots([{ workspace: 'a', path: '/x/y' }], [{ workspace: 'a', path: '/x' }]), [{ workspace: 'a', path: '/x/y' }]);
  assert.deepEqual(intersectRoots([{ workspace: 'a', path: '/x' }], [{ workspace: 'a', path: '/xy' }]), []);
  assert.deepEqual(intersectRoots([{ workspace: 'a', path: '/', ops: ['read'] }], [{ workspace: 'a', path: '/', ops: ['create'] }]), []);

  assert.deepEqual(childGrant({ own }), { roots: own, write_scope: null });
  assert.throws(() => childGrant({ own, parent: [{ workspace: 'content', path: '/' }] }), (e) => e.error_class === 'permission_denied');

  const w = childGrant({ own, writes: [{ workspace: 'automations', path: '/flows/new' }] });
  assert.deepEqual(w.write_scope, [{ workspace: 'automations', path: '/flows/new' }]);
  assert.deepEqual(w.roots[0], { workspace: 'automations', path: '/flows/new', ops: ['read', 'create', 'update', 'delete'] });
  assert.ok(w.roots.slice(1).every((r) => r.ops.length === 1 && r.ops[0] === 'read'), 'outside the writes only reading remains');

  const ro = childGrant({ own, writes: [] });
  assert.deepEqual(ro.write_scope, []);
  assert.ok(ro.roots.length === 2 && ro.roots.every((r) => r.ops.join() === 'read'), 'writes: [] is read-only, never an empty (unconfined) grant');
  assert.throws(() => childGrant({ own, writes: [{ workspace: 'content', path: '/' }] }), (e) => e.error_class === 'invalid_input');
  assert.throws(() => childGrant({ own: [TAGS], writes: [{ workspace: 'tags', path: '/' }] }), (e) => e.error_class === 'invalid_input', 'a read-only root cannot become writable');
});

// ── Spawn of a configured agent ─────────────────────────────────────────────

test('a configured agent\'s child keeps its mode, budgets, on_exceeded and roots', async () => {
  const { spawns, at } = childWorld();
  const env = await spawn({ agent_ref: '/agents/builder', key: 'b', objective: 'Build the reminder', __raisin_context: at(1, FN.spawn) });
  assert.equal(env.status, 'succeeded', JSON.stringify(env.payload));
  const req = spawns[0];
  assert.equal(req.input.config.execution_mode, 'approve_then_auto', 'not forced automatic: a person still approves');
  assert.equal(req.input.config.prior_run_tool_result_chars, 400, 'the agent\'s run_config travels');
  assert.equal(req.reducer.function_path, '/lib/demo/builder-run-step');
  assert.deepEqual(req.budgets, { max_model_calls: 40, max_operations: 240, max_consecutive_op_failures: 6, max_wall_ms: 3600000, max_total_tokens: 300000 });
  assert.equal(req.on_exceeded, 'pause');
  assert.deepEqual(req.objective.allowed_writes, [AUTOMATIONS, TAGS], 'the agent\'s grant goes on the child record');
  assert.deepEqual(req.input.config.node_dev.roots, [AUTOMATIONS, TAGS], 'and to the reducer, as at top level');
  assert.equal(req.input.config.write_scope, undefined);
  assert.equal(env.payload.child.execution_mode, 'approve_then_auto');
});

test('the parent\'s budget request and writes only narrow; the parent\'s grant bounds the child\'s', async () => {
  const { spawns, at } = childWorld({ parentExecutor: { node_dev: { roots: [{ workspace: 'automations', path: '/flows', ops: ['read', 'create'] }] } } });
  const env = await spawn({
    agent_ref: '/agents/builder', key: 'b', objective: 'x',
    budget: { max_model_calls: 500, max_operations: 20, on_exceeded: 'fail' },
    writes: [{ workspace: 'automations', path: '/flows/reminder' }],
    __raisin_context: at(1, FN.spawn),
  });
  assert.equal(env.status, 'succeeded', JSON.stringify(env.payload));
  const req = spawns[0];
  assert.equal(req.budgets.max_model_calls, 40);
  assert.equal(req.budgets.max_operations, 20);
  assert.equal(req.on_exceeded, 'fail');
  assert.deepEqual(req.objective.allowed_writes, [
    { workspace: 'automations', path: '/flows/reminder', ops: ['read', 'create'] },
    { workspace: 'automations', path: '/flows', ops: ['read'] },
  ], 'tags fell outside the parent\'s grant; ops are what both allow');
  assert.deepEqual(req.input.config.write_scope, [{ workspace: 'automations', path: '/flows/reminder' }]);
});

test('a grant that leaves nothing refuses the spawn before any transcript is written', async () => {
  const { f, spawns, at } = childWorld({ parentExecutor: { node_dev: { roots: [{ workspace: 'content', path: '/' }] } } });
  const env = await spawn({ agent_ref: '/agents/builder', key: 'b', objective: 'x', __raisin_context: at(1, FN.spawn) });
  assert.equal(env.payload.error_class, 'permission_denied');
  assert.equal(spawns.length, 0);
  assert.equal(await f.api.nodes.get('ai', '/agents/builder/inbox/chats'), null);
});

test('a copy of the calling agent (no agent_ref) keeps the delegated defaults', async () => {
  const { spawns, at } = childWorld({ leadProps: { execution_mode: 'approve_then_auto', run_budgets: BUILDER_OWN, delegation: { max_depth: 2 } } });
  const env = await spawn({ key: 'self', objective: 'x', __raisin_context: at(1, FN.spawn) });
  assert.equal(env.status, 'succeeded', JSON.stringify(env.payload));
  assert.equal(spawns[0].input.config.execution_mode, 'automatic');
  assert.equal(spawns[0].budgets.max_model_calls, 20);
  assert.equal(spawns[0].on_exceeded, 'fail');
  assert.equal(spawns[0].objective.allowed_writes, undefined, 'core copies the parent\'s own grant');
});

// ── What a child asks for ───────────────────────────────────────────────────

const APPROVAL = {
  request_id: 'req-1', created_seq: 9, requested_by: null, effect_id: 'fx-1',
  kind: { kind: 'approval', subject_digest: 'sha256:abc', digest_alg: 'sha256', summary: 'Create automation "Remind non-responders"', scope: 'build_spec', changes: { artifacts: 1 } },
};
const QUESTION = {
  request_id: 'req-2', created_seq: 10,
  kind: { kind: 'input', prompt: 'Which campaign?', choices: ['Spring', { label: 'Autumn', value: 'autumn' }], schema: null },
};

test('open requests are read from every state that holds them', () => {
  assert.deepEqual(openRequestsOf({ state: { status: 'running', activity: { activity: 'idle', open: [QUESTION] } } }).map((r) => r.prompt), ['Which campaign?']);
  const [a] = openRequestsOf({ state: { status: 'waiting', open: [APPROVAL] } });
  assert.equal(a.kind, 'approval');
  assert.equal(a.needs_person, true);
  assert.equal(a.subject_digest, 'sha256:abc');
  assert.equal(a.summary, 'Create automation "Remind non-responders"');
  assert.deepEqual(openRequestsOf({ state: { status: 'running', activity: { activity: 'operating' } } }), []);
  const ext = openRequestsOf({ state: { status: 'waiting', open: [{ request_id: 'r', kind: { kind: 'external', op_id: 'o', resume_key: 'k', tool: 'browser' } }] } });
  assert.equal(ext[0].needs_person, false);
  assert.ok(!childBlocked({ status: 'waiting', waiting_for: ext }));
});

test('inspect shows what a child waits for; wait answers at once when every pending child is blocked on a person', async () => {
  const { at, waitingOn, running, paused } = childWorld();
  const s = await spawn({ agent_ref: '/agents/builder', key: 'build', objective: 'x', __raisin_context: at(1, FN.spawn) });
  const id = s.payload.child.run_id;
  waitingOn(id, [APPROVAL, QUESTION]);

  const one = await inspectAgent({ agent: 'build', __raisin_context: at(2, FN.inspect) });
  assert.equal(one.payload.child.status, 'waiting');
  assert.deepEqual(one.payload.child.waiting_for.map((r) => r.kind), ['approval', 'input']);
  assert.match(one.payload.needs_attention[0], /Child build is waiting for approval of: Create automation/);
  assert.match(one.payload.needs_attention[0], /an answer to: Which campaign\? \(choices: Spring, Autumn\)/);
  const all = await inspectAgent({ __raisin_context: at(3, FN.inspect) });
  assert.equal(all.payload.needs_attention.length, 1);

  const w = await waitAgents({ __raisin_context: at(4, FN.wait) });
  assert.equal(w.status, 'succeeded', 'a parked wait would wait with the blocked child');
  assert.equal(w.payload.done, false);
  assert.deepEqual(w.payload.pending, ['build']);
  assert.equal(w.payload.children[0].waiting_for[0].request_id, 'req-1');
  assert.match(w.payload.message, /Only a person can approve or answer/);

  // With another child still working, the wait parks on that one and still names the blocked one.
  const r = await spawn({ agent_ref: '/agents/reviewer', key: 'rev', objective: 'y', independent: true, __raisin_context: at(5, FN.spawn) });
  running(r.payload.child.run_id);
  const w2 = await waitAgents({ __raisin_context: at(6, FN.wait) });
  assert.equal(w2.status, 'waiting');
  assert.equal(w2.resume_key, `child:${r.payload.child.run_id}`);
  assert.equal(w2.payload.needs_attention.length, 1);

  // A paused child (on_exceeded: pause) needs someone too.
  paused(r.payload.child.run_id);
  const w3 = await waitAgents({ __raisin_context: at(7, FN.wait) });
  assert.equal(w3.status, 'succeeded');
  assert.equal(w3.payload.needs_attention.length, 2);
  assert.match(w3.payload.needs_attention[1], /Child rev is paused .*max_model_calls.*message-agent resumes it/);
});

test('a finished child carries its artifacts and the evidence it handed back', async () => {
  const { at, runs, finish, running } = childWorld();
  const s = await spawn({ agent_ref: '/agents/builder', key: 'build', objective: 'x', __raisin_context: at(1, FN.spawn) });
  const id = s.payload.child.run_id;
  running(id);
  const insp = await inspectAgent({ agent: 'build', __raisin_context: at(2, FN.inspect) });
  assert.equal(insp.payload.child.evidence, undefined);
  finish(id, 'succeeded', 'Built', [{ locator: { workspace: 'automations', path: '/flows/reminder' } }]);
  runs[id].run.state.outcome.detail.evidence = [{ kind: 'verified', subject: { workspace: 'automations', path: '/flows/reminder' } }];
  const done = await inspectAgent({ agent: 'build', __raisin_context: at(3, FN.inspect) });
  assert.equal(done.payload.child.artifacts[0].locator.path, '/flows/reminder');
  assert.equal(done.payload.child.evidence[0].kind, 'verified');
  assert.equal(done.payload.child.waiting_for, undefined);
});
