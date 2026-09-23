/**
 * Delegation over core's child runs: the typed spawn specification, the
 * agent's delegation policy (depth, fan-out, bounded parallelism, allowed
 * agents), grant narrowing, the child's transcript + brief, the spawn request
 * core receives (objective, grants, budgets, spawn key, the child's own
 * input), inspect / message / interrupt through core, and the wait — which
 * answers at once or parks on core's `child:{id}` resume key.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-delegation.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { childWorld, PARENT, CHAT } from './support/fake-child-runs.mjs';
import {
  normalizeSpawn, childBudgets, narrowTools, acceptance, waitSatisfied, inWriteScope, policyOf, coreObjective, specOfObjective,
} from '../content/functions/lib/raisin/ai/agent-shared/delegation-spec.js';
import { PROJECT_FUNCTION } from '../content/functions/lib/raisin/ai/agent-shared/run-names.js';
import { handler as spawn } from '../content/functions/lib/raisin/ai/spawn-agent/index.js';
import { handler as inspectAgent } from '../content/functions/lib/raisin/ai/inspect-agent/index.js';
import { handler as messageAgent } from '../content/functions/lib/raisin/ai/message-agent/index.js';
import { handler as waitAgents } from '../content/functions/lib/raisin/ai/wait-for-agents/index.js';
import { handler as interruptAgent } from '../content/functions/lib/raisin/ai/interrupt-agent/index.js';
import { handler as delegateTask } from '../content/functions/lib/raisin/ai/delegate-task/index.js';
import { handler as delegationStatus } from '../content/functions/lib/raisin/ai/get-delegation-status/index.js';

const FN = {
  spawn: '/lib/raisin/ai/spawn-agent',
  inspect: '/lib/raisin/ai/inspect-agent',
  message: '/lib/raisin/ai/message-agent',
  wait: '/lib/raisin/ai/wait-for-agents',
  interrupt: '/lib/raisin/ai/interrupt-agent',
  delegate: '/lib/raisin/ai/delegate-task',
  status: '/lib/raisin/ai/get-delegation-status',
};

// ── The typed specification ─────────────────────────────────────────────────

test('the spawn spec is typed, bounded and repairable', () => {
  const s = normalizeSpawn({ objective: { goal: 'Fix typos', done_when: 'no typos' }, context_mode: 'recent', writes: [{ workspace: 'content', path: '/docs' }] });
  assert.equal(s.objective.goal, 'Fix typos');
  assert.equal(s.context_mode, 'recent');
  assert.deepEqual(s.writes, [{ workspace: 'content', path: '/docs' }]);
  assert.equal(s.budget.on_exceeded, 'fail');
  assert.throws(() => normalizeSpawn({}), /objective/);
  assert.throws(() => normalizeSpawn({ objective: 'x', context_mode: 'all' }), /context_mode/);
  assert.throws(() => normalizeSpawn({ objective: 'x', agent_ref: 'rm -rf' }), /agent_ref/);
  assert.throws(() => normalizeSpawn({ objective: 'x', checks: [{ kind: 'vibes' }] }), /checks\[0\]/);
  const b = childBudgets({ max_model_calls: 10000, max_wall_s: 60 });
  assert.equal(b.max_model_calls, 60);
  assert.equal(b.max_wall_ms, 60000);
  assert.deepEqual(policyOf({ delegation: { max_depth: 99, max_parallel: 3 } }).max_depth, 2);
});

test('grants only narrow: a child never gets a tool its agent lacks', () => {
  const tools = [
    { name: 'search', function_path: '/lib/demo/search' },
    { name: 'write', function_path: '/lib/demo/write' },
    { name: 'spawn-agent', function_path: FN.spawn },
  ];
  const r = narrowTools(tools, ['search', 'deploy'], { mayDelegate: false });
  assert.deepEqual(r.tools.map((t) => t.name), ['search']);
  assert.deepEqual(r.refused, ['deploy']);
  assert.deepEqual(narrowTools(tools, null, { mayDelegate: false }).tools.map((t) => t.name), ['search', 'write']);
  assert.ok(inWriteScope([{ workspace: 'c', path: '/docs' }], 'c', '/docs/a'));
  assert.ok(!inWriteScope([{ workspace: 'c', path: '/docs' }], 'c', '/docsx'));
  assert.ok(!inWriteScope([], 'c', '/docs'));
});

test('acceptance is evaluated on stored state, and survives the trip through core\'s objective', () => {
  const spec = normalizeSpawn({
    objective: 'x',
    expected_artifacts: [{ workspace: 'c', path: '/a' }],
    checks: [{ kind: 'property_equals', workspace: 'c', path: '/a', property: 'status', value: 'ok' }, { kind: 'artifact_written', workspace: 'c', path: '/a' }],
  });
  const obj = coreObjective(spec, { brief: 'b', allowedTools: [], contextSelection: { mode: 'none' } });
  assert.ok(obj.acceptance_checks.every((c) => c.required === false && c.id && c.check));
  assert.ok(obj.expected_artifacts.every((a) => a.required === false && a.kind === 'node'));
  const back = specOfObjective(obj);
  assert.deepEqual(back.checks, spec.checks);
  assert.deepEqual(back.expected_artifacts.map((a) => [a.workspace, a.path]), [['c', '/a']]);
  const nodes = new Map([['c:/a', { properties: { status: 'draft' } }]]);
  const r = acceptance(back, { outcome: 'succeeded', written: [{ workspace: 'c', path: '/a' }], nodes });
  assert.equal(r.accepted, false);
  assert.equal(r.checks[0].actual, 'draft');
  nodes.set('c:/a', { properties: { status: 'ok' } });
  assert.equal(acceptance(back, { outcome: 'succeeded', written: [{ workspace: 'c', path: '/a' }], nodes }).accepted, true);
  assert.ok(waitSatisfied('any', [{ status: 'running' }, { status: 'completed' }]));
  assert.ok(!waitSatisfied('all', [{ status: 'running' }, { status: 'completed' }]));
});

// ── Spawn ───────────────────────────────────────────────────────────────────

test('outside a run delegation refuses', async () => {
  childWorld();
  const r = await spawn({ objective: 'x', __raisin_context: { chat_path: CHAT } });
  assert.equal(r.success, false);
  assert.match(r.error, /inside an agent run/);
  const d = await delegateTask({ task_id: 't1', agent_ref: '/agents/reviewer', objective: 'x' });
  assert.equal(d.success, false);
});

test('spawn: core gets the typed objective, grants, budgets and the child\'s own input; the transcript is written first', async () => {
  const { f, spawns, at } = childWorld();
  const env = await spawn({
    agent_ref: '/agents/reviewer', key: 'rev', objective: { goal: 'Review /docs', deliverable: 'a list of typos' },
    context_mode: 'recent', tools: ['search'], writes: [], checks: [{ kind: 'outcome_is', value: 'succeeded' }],
    __raisin_context: at(1, FN.spawn),
  });
  assert.equal(env.status, 'succeeded', JSON.stringify(env.payload));
  const child = env.payload.child;
  assert.equal(child.key, 'rev');
  assert.equal(env.artifact_refs[0].kind, 'child_run');

  const req = spawns[0];
  assert.equal(req.run_id, PARENT);
  assert.equal(req.spawn_key, 'rev');
  assert.equal(req.as_agent, 'functions:/agents/reviewer');
  assert.equal(req.on_exceeded, 'fail');
  assert.equal(req.budgets.on_exceeded, undefined);
  assert.equal(req.reducer.function_path, '/lib/raisin/ai/agent-run-reducer');
  assert.deepEqual(req.objective.allowed_tools, ['/lib/demo/search', PROJECT_FUNCTION]);
  assert.deepEqual(req.objective.allowed_writes, []);
  assert.equal(req.objective.context.mode, 'recent_turns');
  assert.equal(req.objective.context.items[0].text, 'Review the docs and fix typos');
  assert.equal(req.objective.acceptance_checks[0].check.kind, 'outcome_is');
  assert.equal(req.subject.path, child.chat_path);
  assert.equal(req.executor_config.chat_path, child.chat_path);
  assert.deepEqual(req.input.tools.map((t) => t.name), ['search']);
  assert.deepEqual(req.input.config.write_scope, []);
  assert.equal(req.input.context.sender_id, 'user-1');
  assert.equal(req.input.context.delegation.parent_run_id, PARENT);

  assert.match(child.chat_path, /^\/agents\/reviewer\/inbox\/chats\/deleg-/);
  const chat = await f.api.nodes.get('ai', child.chat_path);
  assert.equal(chat.properties.parent_run_id, PARENT);
  assert.deepEqual(chat.properties.participants, ['agent:reviewer'], 'raisin:Conversation requires participants');
  assert.equal(chat.properties.active_agent_run_id, child.run_id);
  const brief = await f.api.nodes.get('ai', `${child.chat_path}/brief`);
  assert.match(brief.properties.content, /Objective: Review \/docs/);
  for (const k of ['body', 'sender_id', 'message_type', 'status']) assert.ok(brief.properties[k], `brief carries ${k}`);
  assert.match(brief.properties.content, /read-only task/);
  assert.match(brief.properties.content, /Review the docs and fix typos/);

  // A re-dispatched spawn gets its child back from core (same spawn key).
  const again = await spawn({ agent_ref: '/agents/reviewer', key: 'rev', objective: 'Review /docs', __raisin_context: at(1, FN.spawn) });
  assert.equal(again.payload.replayed, true);
  assert.equal(again.payload.child.run_id, child.run_id);
});

test('only the run\'s active operation may spawn', async () => {
  const { at } = childWorld();
  const ctx = at(1, FN.spawn);
  const env = await spawn({ objective: 'x', agent_ref: '/agents/reviewer', __raisin_context: { ...ctx, operation_id: `${PARENT}/op/9` } });
  assert.equal(env.status, 'blocked');
  assert.match(env.payload.error, /not the run's active operation/);
});

test('parallel children only for independent work, within the limit', async () => {
  const { at, running } = childWorld();
  const first = await spawn({ agent_ref: '/agents/reviewer', key: 'a', objective: 'A', __raisin_context: at(1, FN.spawn) });
  assert.equal(first.status, 'succeeded');
  running(first.payload.child.run_id);
  const dependent = await spawn({ agent_ref: '/agents/reviewer', key: 'b', objective: 'B', __raisin_context: at(2, FN.spawn) });
  assert.equal(dependent.status, 'failed');
  assert.match(dependent.payload.error, /independent: true/);
  const indep = await spawn({ agent_ref: '/agents/reviewer', key: 'b', objective: 'B', independent: true, __raisin_context: at(3, FN.spawn) });
  assert.equal(indep.status, 'succeeded');
  const third = await spawn({ agent_ref: '/agents/reviewer', key: 'c', objective: 'C', independent: true, __raisin_context: at(4, FN.spawn) });
  assert.equal(third.payload.error_class, 'conflict');
  assert.match(third.payload.error, /limit 2 at once/);
});

test('depth (from the run record) and allowed agents are enforced; a child at the limit gets no delegation tools', async () => {
  const deep = childWorld({ depth: 1 });
  const env = await spawn({ agent_ref: '/agents/reviewer', objective: 'x', __raisin_context: deep.at(1, FN.spawn) });
  assert.equal(env.status, 'blocked');
  assert.match(env.payload.error, /may not delegate further/);

  const w = childWorld({ leadProps: { delegation: { allowed_agents: ['/agents/writer'] } } });
  const refused = await spawn({ agent_ref: '/agents/reviewer', objective: 'x', __raisin_context: w.at(1, FN.spawn) });
  assert.equal(refused.payload.error_class, 'permission_denied');

  const ok = childWorld();
  await spawn({ agent_ref: '/agents/reviewer', objective: 'x', __raisin_context: ok.at(1, FN.spawn) });
  assert.ok(!ok.spawns[0].input.tools.some((t) => t.function_path === FN.spawn));
  assert.ok(!ok.spawns[0].objective.allowed_tools.includes(FN.spawn));
});

// ── Inspect, message, interrupt ─────────────────────────────────────────────

test('inspect, message (transcript first, then core) and interrupt a child', async () => {
  const { f, at, running, childControls, post } = childWorld();
  const s = await spawn({ agent_ref: '/agents/reviewer', key: 'rev', objective: 'x', __raisin_context: at(1, FN.spawn) });
  const childId = s.payload.child.run_id;
  running(childId);
  post(childId, { text: 'found the FAQ too' });

  const all = await inspectAgent({ __raisin_context: at(2, FN.inspect) });
  assert.equal(all.payload.children.length, 1);
  assert.equal(all.payload.live, 1);
  assert.deepEqual(all.payload.messages.map((m) => m.message.text), ['found the FAQ too']);
  const again = await inspectAgent({ agent: 'rev', __raisin_context: at(3, FN.inspect) });
  assert.equal(again.payload.messages, undefined, 'read messages are acknowledged');
  assert.equal(again.payload.child.recent_events.length, 1);

  const m = await messageAgent({ agent: 'rev', text: 'also check /faq', mode: 'steer', __raisin_context: at(4, FN.message) });
  assert.equal(m.status, 'succeeded', JSON.stringify(m.payload));
  assert.equal(m.payload.state, 'queued');
  const ctl = childControls.at(-1);
  assert.equal(ctl.run_id, PARENT);
  assert.equal(ctl.child_run_id, childId);
  assert.equal(ctl.action, 'steer');
  assert.match(ctl.input.text, /Redirect/);
  const written = await f.api.nodes.get('ai', ctl.input.message_path);
  assert.equal(written.properties.run_steer_state, 'queued');
  assert.equal(written.properties.agent_run_id, childId);

  const missing = await inspectAgent({ agent: 'nope', __raisin_context: at(5, FN.inspect) });
  assert.equal(missing.payload.error_class, 'not_found');

  const i = await interruptAgent({ all: true, reason: 'enough', __raisin_context: at(6, FN.interrupt) });
  assert.equal(i.payload.interrupted, 1);
  assert.equal(childControls.at(-1).action, 'interrupt');
  assert.equal(childControls.at(-1).mode, 'stop');
  const done = await messageAgent({ agent: 'rev', text: 'hello?', __raisin_context: at(7, FN.message) });
  assert.equal(done.payload.error_class, 'conflict');
});

// ── Wait ────────────────────────────────────────────────────────────────────

test('wait parks on core\'s resume key for the next live child; answers once they are done', async () => {
  const { at, running, finish } = childWorld();
  const a = await spawn({ agent_ref: '/agents/reviewer', key: 'a', objective: 'A', checks: [{ kind: 'outcome_is', value: 'succeeded' }], __raisin_context: at(1, FN.spawn) });
  const b = await spawn({ agent_ref: '/agents/reviewer', key: 'b', objective: 'B', independent: true, __raisin_context: at(2, FN.spawn) });
  const [ida, idb] = [a.payload.child.run_id, b.payload.child.run_id];
  running(ida);
  running(idb);

  const w = await waitAgents({ __raisin_context: at(3, FN.wait) });
  assert.equal(w.status, 'waiting');
  assert.equal(w.resume_key, `child:${ida}`);
  assert.deepEqual(w.payload.waiting_for, ['a', 'b']);

  finish(ida, 'succeeded', 'Found 3 typos');
  const w2 = await waitAgents({ __raisin_context: at(4, FN.wait) });
  assert.equal(w2.status, 'waiting');
  assert.equal(w2.resume_key, `child:${idb}`);

  finish(idb, 'partial', 'half');
  const w3 = await waitAgents({ __raisin_context: at(5, FN.wait) });
  assert.equal(w3.status, 'succeeded');
  assert.equal(w3.payload.done, true);
  assert.equal(w3.payload.finished, 2);
  const ca = w3.payload.children.find((c) => c.key === 'a');
  assert.equal(ca.summary, 'Found 3 typos');
  assert.equal(ca.accepted, true);
  assert.equal(ca.checks[0].ok, true);
  assert.equal(w3.payload.children.find((c) => c.key === 'b').accepted, false);
});

test('mode any answers as soon as one named child is done', async () => {
  const { at, running, finish } = childWorld();
  const a = await spawn({ agent_ref: '/agents/reviewer', key: 'a', objective: 'A', __raisin_context: at(1, FN.spawn) });
  const b = await spawn({ agent_ref: '/agents/reviewer', key: 'b', objective: 'B', independent: true, __raisin_context: at(2, FN.spawn) });
  running(b.payload.child.run_id);
  finish(a.payload.child.run_id, 'succeeded', 'ok');
  const w = await waitAgents({ mode: 'any', __raisin_context: at(3, FN.wait) });
  assert.equal(w.status, 'succeeded');
  assert.deepEqual(w.payload.pending, ['b']);
  const none = await waitAgents({ agents: ['zzz'], __raisin_context: at(4, FN.wait) });
  assert.equal(none.payload.error_class, 'not_found');
});

// ── The plan-task spellings ─────────────────────────────────────────────────

test('delegate-task is spawn keyed by task: the same task finds its first child', async () => {
  const { spawns, at, finish } = childWorld();
  const env = await delegateTask({ task_id: 't1', agent_ref: '/agents/reviewer', objective: 'Review', context: { area: 'docs' }, __raisin_context: at(1, FN.delegate) });
  assert.equal(env.status, 'succeeded', JSON.stringify(env.payload));
  assert.equal(env.payload.child.key, 'task-t1');
  assert.equal(env.payload.child.task_id, 't1');
  assert.equal(spawns[0].objective.context.mode, 'snapshot');
  assert.match(spawns[0].input.text, /Structured snapshot/);
  const dup = await delegateTask({ task_id: 't1', agent_ref: '/agents/reviewer', objective: 'Review', __raisin_context: at(2, FN.delegate) });
  assert.equal(dup.payload.replayed, true);
  assert.equal(dup.payload.child_run_id, env.payload.child_run_id);

  finish(env.payload.child_run_id, 'succeeded', 'Reviewed');
  const st = await delegationStatus({ task_id: 't1', __raisin_context: at(3, FN.status) });
  assert.equal(st.status, 'succeeded', JSON.stringify(st.payload));
  assert.equal(st.payload.status, 'completed');
  assert.equal(st.payload.result, 'Reviewed');
});
