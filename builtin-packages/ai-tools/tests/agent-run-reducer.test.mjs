/**
 * The generic agent-loop reducer against the reducer contract: effect ids,
 * one operation per response, terminal exclusivity, R10 and R12, the seq
 * guard, the plan as run state, completion from evidence only, approval,
 * steering, loop detection and the no-progress stop.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-run-reducer.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { handler } from '../content/functions/lib/raisin/ai/agent-run-reducer/index.js';
import { PROJECT_FUNCTION } from '../content/functions/lib/raisin/ai/agent-shared/run-names.js';

const OPS = new Set(['call_tool', 'request_model_turn', 'request_approval', 'ask_user']);

/* AGENT_RUN_DUMP=<file>: record every (request, response) so the Rust
 * `validate_response` can check the same transcripts. */
const DUMP = process.env.AGENT_RUN_DUMP ? [] : null;
if (DUMP) {
  const { writeFileSync } = await import('node:fs');
  process.on('exit', () => writeFileSync(process.env.AGENT_RUN_DUMP, JSON.stringify(DUMP)));
}

/** The contract rules a response must satisfy (a JS mirror of validate_response). */
function validate(req, resp) {
  if (DUMP) DUMP.push({ request: JSON.parse(JSON.stringify(req)), response: JSON.parse(JSON.stringify(resp)) });
  assert.equal(resp.contract, 'raisin.agent-run.reducer/1');
  assert.ok([req.state_rev, req.state_rev + 1].includes(resp.state_rev), 'R2');
  resp.effects.forEach((e, i) => assert.equal(e.effect_id, `${resp.state_rev}:${i}`, 'R3'));
  assert.ok(resp.effects.filter((e) => OPS.has(e.kind)).length <= 1, 'R4 one operation');
  const terminal = resp.effects.some((e) => e.kind === 'complete' || e.kind === 'fail');
  if (terminal) assert.ok(!resp.effects.some((e) => OPS.has(e.kind)), 'R5');
  if (resp.effects.length) assert.equal(resp.state_rev, req.state_rev + 1, 'R10');
  for (const e of resp.effects) {
    if (e.kind === 'request_model_turn') {
      const given = new Set(e.tool_results.map((r) => r.call_id));
      for (const id of req.run.unanswered_calls) assert.ok(given.has(id), `R12 ${id}`);
    }
  }
  assert.ok(JSON.stringify(resp.state).length < 256 * 1024, 'R9');
}

/** A tiny core: feeds events, tracks rev/seq/unanswered calls/open requests. */
function sim(input) {
  const s = { seq: 0, rev: 0, state: null, unanswered: [], open: [], status: 'running', log: [] };
  s.send = async (kind, data = {}, extra = {}) => {
    s.seq += 1;
    const req = {
      contract: 'raisin.agent-run.reducer/1', accept: ['raisin.agent-run.reducer/1'],
      run: { run_id: 'run-1', status: s.status, turn: 1, last_seq: s.seq, usage: {}, budgets: {}, open_requests: s.open, unanswered_calls: s.unanswered, subject: { workspace: 'ai', path: '/agents/a/inbox/chats/c' } },
      state: s.state, state_rev: s.rev, event: { seq: s.seq, kind, data, ...extra },
    };
    const resp = await handler(req);
    validate(req, resp);
    s.state = resp.state;
    s.rev = resp.state_rev;
    s.last = resp;
    for (const e of resp.effects) {
      if (e.kind === 'request_model_turn') s.unanswered = [];
      if (e.kind === 'request_approval') s.open = [{ request_id: 'run-1/req/1', kind: 'approval', effect_id: e.effect_id, subject_digest: e.subject.digest }];
      if (e.kind === 'withdraw_request') s.open = s.open.filter((r) => r.request_id !== e.request_id);
      if (e.kind === 'complete' || e.kind === 'fail') s.status = e.kind === 'complete' ? 'completed' : 'failed';
    }
    s.log.push(resp);
    return resp;
  };
  s.op = () => s.last.effects.find((e) => OPS.has(e.kind));
  s.modelDone = async (text, calls = [], op = 'run-1/op/1') => {
    const eff = s.op();
    assert.equal(eff.kind, 'request_model_turn');
    s.unanswered = calls.map((c) => c.call_id);
    return s.send('model_turn_completed', { message: { text }, tool_calls: calls, finish_reason: calls.length ? 'tool_calls' : 'stop' }, { effect_id: eff.effect_id, operation_id: op });
  };
  s.toolDone = async (envelope, op = 'run-1/op/2') => {
    const eff = s.op();
    assert.equal(eff.kind, 'call_tool');
    return s.send('tool_result', { call_id: eff.for_call_id, tool: eff.tool, envelope }, { effect_id: eff.effect_id, operation_id: op });
  };
  s.start = () => s.send('run_started', { input });
  return s;
}

const TOOLS = [
  { name: 'search', kind: 'function', function_path: '/lib/demo/search', mutating: false, replay_safe: true },
  { name: 'create-plan', kind: 'domain', domain_op: 'plan.create' },
  { name: 'update-task', kind: 'domain', domain_op: 'plan.update_task' },
];
const ok = (payload = { hits: 1 }) => ({ envelope: 'raisin.tool-result/1', operation_id: 'x', status: 'succeeded', payload, writes: [] });

test('a turn with a tool call runs it, feeds the result back, and finishes through the projection', async () => {
  const s = sim({ text: 'find it', tools: TOOLS, context: { workspace: 'ai', chat_path: '/agents/a/inbox/chats/c' } });
  const r0 = await s.start();
  assert.equal(r0.effects[0].kind, 'request_model_turn');
  assert.deepEqual(r0.effects[0].tools_offered.map((t) => t.kind), ['function', 'domain', 'domain']);
  const r1 = await s.modelDone('', [{ call_id: 'c1', name: 'search', args: { q: 'x' } }]);
  const call = r1.effects[0];
  assert.equal(call.kind, 'call_tool');
  assert.equal(call.tool, '/lib/demo/search');
  assert.equal(call.for_call_id, 'c1');
  assert.equal(call.args.__raisin_context.chat_path, '/agents/a/inbox/chats/c');
  assert.equal(call.args.__raisin_context.msg_path, '/agents/a/inbox/chats/c/run-run1-op-1');
  const r2 = await s.toolDone({ legacy: true, status: 'succeeded', payload: { hits: 3 } });
  assert.equal(r2.effects[0].kind, 'request_model_turn');
  assert.deepEqual(r2.effects[0].tool_results, [{ call_id: 'c1', synthetic: false, content: { hits: 3 } }]);
  const r3 = await s.modelDone('Found three.', [], 'run-1/op/3');
  assert.equal(r3.effects[0].kind, 'call_tool');
  assert.equal(r3.effects[0].tool, PROJECT_FUNCTION);
  assert.equal(r3.effects[0].args.reason, 'final');
  const r4 = await s.toolDone(ok({ projected: true }), 'run-1/op/4');
  assert.equal(r4.effects[0].kind, 'complete');
  assert.equal(r4.effects[0].outcome, 'succeeded');
});

test('the seq guard makes a re-delivery a no-op', async () => {
  const s = sim({ text: 'x', tools: TOOLS });
  await s.start();
  const before = s.rev;
  const replay = await handler({
    contract: 'raisin.agent-run.reducer/1', accept: ['raisin.agent-run.reducer/1'],
    run: { run_id: 'run-1', status: 'running', last_seq: 1, unanswered_calls: [], open_requests: [] },
    state: s.state, state_rev: s.rev, event: { seq: 1, kind: 'run_started', data: {} },
  });
  assert.equal(replay.state_rev, before);
  assert.deepEqual(replay.effects, []);
});

test('completion is REQUESTED by the model and granted only with evidence', async () => {
  const s = sim({ text: 'do it', tools: TOOLS });
  await s.start();
  // Plan + start task 1 + claim it done in the same turn, with no tool run.
  await s.modelDone('', [
    { call_id: 'p', name: 'create_plan', args: { title: 'P', tasks: [{ title: 'one' }, { title: 'two' }] } },
    { call_id: 'u1', name: 'update_task', args: { task_id: 't1', status: 'in_progress' } },
    { call_id: 'u2', name: 'update_task', args: { task_id: 't1', status: 'completed' } },
  ]);
  const refusal = s.state.results.find((r) => r.call_id === 'u2').content;
  assert.equal(refusal.success, false);
  assert.equal(refusal.refused, 'no_evidence');
  assert.equal(s.state.plan.tasks[0].status, 'in_progress');
  // The plan changed: projection first, then the model turn.
  assert.equal(s.op().tool, PROJECT_FUNCTION);
  await s.toolDone(ok());
  // Now do real work, then request completion again.
  await s.modelDone('', [{ call_id: 's', name: 'search', args: { q: 'y' } }], 'run-1/op/3');
  await s.toolDone(ok(), 'run-1/op/4');
  assert.equal(s.state.plan.tasks[0].evidence.length, 1);
  await s.modelDone('', [{ call_id: 'u3', name: 'update_task', args: { task_id: 't1', status: 'completed' } }], 'run-1/op/5');
  assert.equal(s.state.plan.tasks[0].status, 'completed');
  // Projection reports t1 completed, t2 pending.
  const proj = s.last.projection;
  assert.deepEqual(proj.items.map((i) => i.status), ['completed', 'pending']);
});

test('a finished run with open tasks is honest: partial, with the reason', async () => {
  const s = sim({ text: 'do it', tools: TOOLS, config: { execution_mode: 'manual' } });
  await s.start();
  await s.modelDone('', [{ call_id: 'p', name: 'create_plan', args: { title: 'P', tasks: [{ title: 'one' }] } }]);
  await s.toolDone(ok());
  await s.modelDone('All done!', [], 'run-1/op/3');
  await s.toolDone(ok(), 'run-1/op/4');
  const done = s.last.effects[0];
  assert.equal(done.kind, 'complete');
  assert.equal(done.outcome, 'partial');
  assert.match(done.summary, /Done: 0 of 1 steps/);
});

test('an approval-gated plan waits, and approval resumes it with the digest bound', async () => {
  const s = sim({ text: 'plan it', tools: TOOLS, config: { execution_mode: 'approve_then_auto', requires_approval: true } });
  await s.start();
  await s.modelDone('', [
    { call_id: 'p', name: 'create-plan', args: { title: 'P', tasks: [{ title: 'one' }] } },
    { call_id: 's', name: 'search', args: {} },
  ]);
  // Projection first (the card shows "pending approval"), then the request.
  assert.equal(s.op().tool, PROJECT_FUNCTION);
  assert.equal(s.op().args.reason, 'approval');
  await s.toolDone(ok());
  const req = s.last.effects[0];
  assert.equal(req.kind, 'request_approval');
  assert.equal(req.subject.kind, 'plan');
  assert.ok(req.subject.digest);
  // The search in the same turn was answered, never run.
  assert.equal(s.state.results.find((r) => r.call_id === 's').content.status, 'refused');
  s.open = [];
  await s.send('request_resolved', { request_id: 'run-1/req/1', kind: 'approval', decision: 'approve', subject_digest: req.subject.digest }, { effect_id: req.effect_id });
  // plan dirty → projection, then the model turn carrying the approval answer.
  await s.toolDone(ok(), 'run-1/op/3');
  const turn = s.op();
  assert.equal(turn.kind, 'request_model_turn');
  const answer = turn.tool_results.find((r) => r.call_id === 'p').content;
  assert.equal(answer.status, 'approved');
});

test('agent_decides: a plan the agent asks approval for waits; one it does not ask for runs', async () => {
  const asked = sim({ text: 'plan it', tools: TOOLS, config: { execution_mode: 'agent_decides', approval_optional: true } });
  await asked.start();
  await asked.modelDone('', [{ call_id: 'p', name: 'create_plan', args: { title: 'P', tasks: [{ title: 'one' }], ask_approval: true } }]);
  await asked.toolDone(ok());
  assert.equal(asked.last.effects[0].kind, 'request_approval');
  assert.equal(asked.state.plan.status, 'pending_approval');

  const free = sim({ text: 'plan it', tools: TOOLS, config: { execution_mode: 'agent_decides', approval_optional: true } });
  await free.start();
  await free.modelDone('', [{ call_id: 'p', name: 'create_plan', args: { title: 'P', tasks: [{ title: 'one' }] } }]);
  assert.equal(free.state.plan.status, 'active');
  assert.ok(!free.last.effects.some((e) => e.kind === 'request_approval'));
});

test('ask_approval is ignored outside agent_decides', async () => {
  const s = sim({ text: 'plan it', tools: TOOLS, config: { execution_mode: 'automatic' } });
  await s.start();
  await s.modelDone('', [{ call_id: 'p', name: 'create_plan', args: { title: 'P', tasks: [{ title: 'one' }], ask_approval: true } }]);
  assert.equal(s.state.plan.status, 'active');
});

test('a steer while waiting for approval withdraws the request and asks the model', async () => {
  const s = sim({ text: 'plan it', tools: TOOLS, config: { requires_approval: true } });
  await s.start();
  await s.modelDone('', [{ call_id: 'p', name: 'create_plan', args: { title: 'P', tasks: [{ title: 'one' }] } }]);
  await s.toolDone(ok());
  assert.equal(s.last.effects[0].kind, 'request_approval');
  const r = await s.send('user_input', { steer_id: 'run-1/steer/1', input: { text: 'change it' } });
  assert.equal(r.effects[0].kind, 'withdraw_request');
  assert.ok(r.effects.some((e) => e.kind === 'call_tool' || e.kind === 'request_model_turn'));
  assert.equal(s.state.plan.status, 'rejected');
});

test('an identical call past the loop limit is refused, and repeated loops block the run', async () => {
  const WRITER = [{ name: 'search', kind: 'function', function_path: '/lib/demo/search', mutating: true, replay_safe: false }];
  const s = sim({ text: 'x', tools: WRITER, config: { loop_limit: 1, loop_hits_limit: 2 } });
  await s.start();
  let op = 1;
  const next = () => `run-1/op/${++op}`;
  await s.modelDone('', [{ call_id: 'a', name: 'search', args: { q: 'same' } }], 'run-1/op/1');
  await s.send('operation_failed', { error_class: 'tool_error', message: 'boom', call_id: 'a' }, { effect_id: s.op().effect_id, operation_id: next() });
  await s.modelDone('', [{ call_id: 'b', name: 'search', args: { q: 'same' } }], next());
  assert.equal(s.state.results.find((r) => r.call_id === 'b').content.loop_detected, true);
  assert.equal(s.op().kind, 'request_model_turn');
  assert.match(s.op().instructions, /already run 1 time/);
  await s.modelDone('', [{ call_id: 'c', name: 'search', args: { q: 'same' } }], next());
  // Second loop hit with nothing else to do: one tools-free summary turn, then finish blocked.
  assert.equal(s.op().kind, 'request_model_turn');
  assert.deepEqual(s.op().tools_offered, []);
  assert.match(s.op().instructions, /repeating the same step/);
  assert.equal(s.op().context.restate_instructions, true);
  await s.modelDone('I got stuck repeating a search.', [], next());
  assert.equal(s.op().args.reason, 'final');
  assert.equal(s.state.finish.reason, 'loop_detected');
  await s.toolDone(ok(), next());
  assert.equal(s.last.effects[0].outcome, 'blocked');
});

test('a read-only tool may repeat past the loop limit; a successful write clears what was counted', async () => {
  const MIXED = [
    { name: 'search', kind: 'function', function_path: '/lib/demo/search', mutating: false, replay_safe: true },
    { name: 'write', kind: 'function', function_path: '/lib/demo/write', mutating: true, replay_safe: false },
  ];
  const s = sim({ text: 'x', tools: MIXED, config: { loop_limit: 1, loop_hits_limit: 5 } });
  await s.start();
  let op = 1;
  const next = () => `run-1/op/${++op}`;
  for (const id of ['a', 'b']) {
    await s.modelDone('', [{ call_id: id, name: 'search', args: { q: 'same' } }], next());
    assert.equal(s.op().kind, 'call_tool', 'the read runs again');
    await s.toolDone(ok(), next());
  }
  assert.equal(s.state.progress.loop_hits, 0);
  // Twice in ONE turn still runs once.
  await s.modelDone('', [{ call_id: 'c', name: 'search', args: { q: 'z' } }, { call_id: 'd', name: 'search', args: { q: 'z' } }], next());
  assert.equal(s.state.results.find((r) => r.call_id === 'd').content.loop_detected, true);
  await s.toolDone(ok(), next());
  // A successful write clears the counts: the same write is allowed again.
  await s.modelDone('', [{ call_id: 'w1', name: 'write', args: { v: 1 } }], next());
  await s.toolDone(ok(), next());
  assert.deepEqual(s.state.fingerprints, {});
  await s.modelDone('', [{ call_id: 'w2', name: 'write', args: { v: 1 } }], next());
  assert.equal(s.op().kind, 'call_tool');
});

test('turns without a successful tool call end the run as blocked', async () => {
  const s = sim({ text: 'x', tools: TOOLS, config: { no_progress_limit: 2 } });
  await s.start();
  let op = 1;
  const next = () => `run-1/op/${++op}`;
  for (const q of ['a', 'b']) {
    await s.modelDone('', [{ call_id: q, name: 'search', args: { q } }], next());
    const eff = s.op();
    await s.send('operation_failed', { error_class: 'tool_error', message: 'boom', call_id: q }, { effect_id: eff.effect_id, operation_id: next() });
  }
  // The model gets one tools-free turn to say why, then the run ends blocked.
  assert.equal(s.op().kind, 'request_model_turn');
  assert.deepEqual(s.op().tools_offered, []);
  assert.equal(s.state.pending_stop.reason, 'no_progress');
  await s.modelDone('The searches kept failing.', [], next());
  assert.equal(s.op().args.reason, 'final');
  assert.equal(s.state.finish.outcome, 'blocked');
  assert.equal(s.state.finish.reason, 'no_progress');
  assert.equal(s.state.finish.summarised, true);
  assert.equal(s.state.pending_stop, null);
});

test('a message that arrives while a stalled run is finishing gets a real model turn, not a final-projection loop', async () => {
  const s = sim({ text: 'x', tools: TOOLS, config: { no_progress_limit: 2 } });
  await s.start();
  let op = 1;
  const next = () => `run-1/op/${++op}`;
  for (const q of ['a', 'b']) {
    await s.modelDone('', [{ call_id: q, name: 'search', args: { q } }], next());
    const eff = s.op();
    await s.send('operation_failed', { error_class: 'tool_error', message: 'boom', call_id: q }, { effect_id: eff.effect_id, operation_id: next() });
  }
  await s.modelDone('The searches kept failing.', [], next());
  const proj = s.op();
  assert.equal(proj.args.reason, 'final');
  // The person writes while the final projection runs ("??").
  await s.send('user_input', { steer_id: 'run-1/steer/1', input: { text: '??' } });
  await s.send('tool_result', { envelope: ok() }, { effect_id: proj.effect_id, operation_id: next() });
  const after = s.op();
  assert.equal(after.kind, 'request_model_turn', 'the steer is answered by the model');
  assert.equal(s.state.finish, null);
  assert.equal(s.state.steer_pending, false);
});

test('a retryable model failure retries, then fails the run through the projection', async () => {
  const s = sim({ text: 'x', tools: TOOLS, config: { model_retry_limit: 1 } });
  await s.start();
  const fail = async () => {
    const eff = s.op();
    return s.send('operation_failed', { error_class: 'provider', retryable: true, message: '503' }, { effect_id: eff.effect_id, operation_id: 'run-1/op/1' });
  };
  await fail();
  assert.equal(s.op().kind, 'request_model_turn');
  await fail();
  assert.equal(s.op().args.reason, 'final');
  await s.toolDone(ok());
  assert.equal(s.last.effects[0].kind, 'fail');
  assert.equal(s.last.effects[0].code, 'model_turn_failed');
});

test('after stop the reducer only ingests: a late write is recorded, only a checkpoint is emitted', async () => {
  const s = sim({ text: 'x', tools: TOOLS });
  await s.start();
  await s.modelDone('', [{ call_id: 'a', name: 'search', args: {} }]);
  s.status = 'stopped';
  const eff = s.op();
  const late = await s.send('tool_result', { envelope: { envelope: 'raisin.tool-result/1', status: 'succeeded', writes: [{ locator: { workspace: 'ai', path: '/x' }, action: 'created' }] } }, { effect_id: eff.effect_id, operation_id: 'run-1/op/2' });
  assert.deepEqual(late.effects, []);
  assert.ok(s.state.writes.some((w) => w.path === '/x' && w.late));
  const stopped = await s.send('stopped', { reason: 'user' });
  assert.deepEqual(stopped.effects.map((e) => e.kind), ['checkpoint']);
});

test('a result for an effect the run is not awaiting changes nothing', async () => {
  const s = sim({ text: 'x', tools: TOOLS });
  await s.start();
  const r = await s.send('tool_result', { envelope: {} }, { effect_id: '99:0', operation_id: 'run-1/op/9' });
  assert.deepEqual(r.effects, []);
  assert.equal(s.state.diagnostics.at(-1).code, 'stale_result');
});

test('a tool the model was not offered is refused, never run', async () => {
  const s = sim({ text: 'x', tools: TOOLS });
  await s.start();
  await s.modelDone('', [{ call_id: 'z', name: 'delete_everything', args: {} }]);
  assert.equal(s.op().kind, 'request_model_turn');
  assert.match(s.op().tool_results[0].content.error, /not a tool offered/);
});

// ── Plans that stay one plan, honest stops ──────────────────────────────────

const PLAN_TOOLS = TOOLS.concat([
  { name: 'add-task', kind: 'domain', domain_op: 'plan.add_task' },
  { name: 'write', kind: 'function', function_path: '/lib/demo/write', mutating: true, replay_safe: false, description: 'Write something' },
]);
let opNo = 100;
const nextOp = () => `run-1/op/${++opNo}`;
/** Answer any projection the reducer issues first. */
async function settle(s) {
  while (s.op() && s.op().kind === 'call_tool' && s.op().tool === PROJECT_FUNCTION && s.op().args.reason !== 'final') {
    await s.toolDone(ok(), nextOp());
  }
}
const createPlan = (id, title, extra = {}) => ({ call_id: id, name: 'create_plan', args: { title, tasks: [{ title: 'one' }, { title: 'two' }], ...extra } });
const resultOf = (s, id) => s.state.results.find((r) => r.call_id === id).content;

test('create_plan while a plan has open steps is refused with the open steps listed', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A')]);
  await settle(s);
  await s.modelDone('', [createPlan('p2', 'B')], nextOp());
  const r = resultOf(s, 'p2');
  assert.equal(r.success, false);
  assert.equal(r.error, 'plan_exists');
  assert.equal(r.message, 'A plan with open steps already exists. Continue it (update_task), add to it (add_task), or pass replace: true to start over.');
  assert.deepEqual(r.tasks.map((t) => t.task_id), ['t1', 't2']);
  assert.equal(s.state.plan.title, 'A');
  assert.equal(s.op().kind, 'request_model_turn', 'nothing changed, nothing to project');
});

test('create_plan with replace closes the old card as cancelled in the same projection', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A')]);
  await settle(s);
  await s.modelDone('', [createPlan('p2', 'B', { replace: true })], nextOp());
  assert.equal(s.state.plan.title, 'B');
  const proj = s.op();
  assert.equal(proj.tool, PROJECT_FUNCTION);
  assert.equal(proj.args.plan.title, 'B');
  assert.equal(proj.args.closed_plans.length, 1);
  assert.equal(proj.args.closed_plans[0].status, 'cancelled');
  assert.equal(proj.args.closed_plans[0].title, 'A');
  assert.equal(s.state.closed_plans.length, 0, 'delivered once');
});

test('add_task with an existing title returns that task', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A')]);
  await settle(s);
  await s.modelDone('', [{ call_id: 'a', name: 'add_task', args: { title: '  ONE ' } }], nextOp());
  assert.deepEqual(resultOf(s, 'a'), { success: true, duplicate: true, task_id: 't1', status: 'pending', message: 'That step already exists.' });
  assert.equal(s.state.plan.tasks.length, 2);
});

test('a plan carried from an earlier run is labelled, and is not nudged until the model touches it', async () => {
  const carried = { no: 1, title: 'Old', status: 'active', card_path: '/c/plan', tasks: [{ key: 't1', title: 'one', status: 'pending' }] };
  const s = sim({ text: 'something else', tools: PLAN_TOOLS, plan: carried, config: { execution_mode: 'automatic' } });
  const r0 = await s.start();
  assert.equal(r0.effects[0].context.facts.plan.carried, true);
  await s.modelDone('Done with the new thing.');
  assert.equal(s.op().args.reason, 'final', 'no nudge for a carried plan');

  const fresh = sim({ text: 'x', tools: PLAN_TOOLS, plan: carried, config: { execution_mode: 'automatic' } });
  await fresh.start();
  await fresh.modelDone('', [{ call_id: 'u', name: 'update_task', args: { task_id: 't1', status: 'in_progress' } }]);
  await settle(fresh);
  assert.equal(fresh.state.plan.touched, true);
  await fresh.modelDone('stopping', [], nextOp());
  assert.equal(fresh.op().kind, 'request_model_turn', 'once engaged, the open plan is nudged');
  assert.equal(fresh.op().context.restate_instructions, true);
  assert.match(fresh.op().instructions, /open tasks/);
});

test('renderFacts says a carried plan may be unrelated', async () => {
  const { renderFacts } = await import('../content/functions/lib/raisin/ai/agent-run-model-turn/context.js');
  const text = renderFacts({ plan: { title: 'Old', status: 'active', carried: true, tasks: [] } });
  assert.match(text, /carried over from earlier in this conversation; the new request may be unrelated: continue it, or replace it with create_plan replace: true/);
  assert.doesNotMatch(renderFacts({ recent_writes: [{ action: 'created', workspace: 'w', path: '/p' }] }), /only paths that exist/);
});

test('a carried plan that was waiting for approval has its card closed as cancelled', async () => {
  const carried = { no: 3, title: 'Old', status: 'pending_approval', card_path: '/c/plan', tasks: [{ key: 't1', title: 'one', status: 'pending' }] };
  const s = sim({ text: 'hello', tools: PLAN_TOOLS, plan: carried });
  const r = await s.start();
  assert.equal(r.effects[0].tool, PROJECT_FUNCTION);
  assert.equal(r.effects[0].args.plan, null);
  assert.equal(r.effects[0].args.closed_plans[0].status, 'cancelled');
  assert.equal(r.effects[0].args.closed_plans[0].card_path, '/c/plan');
  await s.toolDone(ok(), nextOp());
  assert.equal(s.op().kind, 'request_model_turn');
  assert.equal(s.state.plan, null);
});

test('a tool success with no task in progress starts the first pending task and gives it the evidence', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A')]);
  await settle(s);
  await s.modelDone('', [{ call_id: 'w', name: 'write', args: { v: 1 } }], nextOp());
  await s.toolDone(ok(), nextOp());
  const t1 = s.state.plan.tasks[0];
  assert.equal(t1.status, 'in_progress');
  assert.equal(t1.evidence.length, 1);
  assert.equal(t1.last_action, 'write');
  assert.equal(s.op().tool, PROJECT_FUNCTION, 'the card is projected with the task in progress');
  assert.equal(s.op().args.plan.tasks[0].status, 'in_progress');
});

test('plan card tasks carry notes, description and the last action', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  await s.modelDone('', [{ call_id: 'p', name: 'create_plan', args: { title: 'A', tasks: [{ title: 'one', description: 'the first' }] } }]);
  await settle(s);
  await s.modelDone('', [{ call_id: 'u', name: 'update_task', args: { task_id: 't1', status: 'in_progress', notes: 'started' } }], nextOp());
  await s.send('tool_result', { envelope: ok() }, { effect_id: s.op().effect_id, operation_id: nextOp() });
  await s.modelDone('', [{ call_id: 'w', name: 'write', args: { v: 1 } }], nextOp());
  await s.toolDone(ok(), nextOp());
  await s.modelDone('', [{ call_id: 'u2', name: 'update_task', args: { task_id: 't1', status: 'completed' } }], nextOp());
  const card = s.op().args.plan.tasks[0];
  assert.equal(card.notes, 'started');
  assert.equal(card.description, 'the first');
  assert.equal(card.last_action, 'write');
});

test('completion refusals escalate: a clearer instruction the second time, the summary path the third', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A'), { call_id: 'u0', name: 'update_task', args: { task_id: 't1', status: 'in_progress' } }]);
  await settle(s);
  const ask = async (id) => {
    await s.modelDone('', [{ call_id: id, name: 'update_task', args: { task_id: 't1', status: 'completed' } }], nextOp());
  };
  await ask('u1');
  const first = resultOf(s, 'u1').message;
  assert.match(first, /a read that confirms the result counts/);
  assert.match(first, /do not repeat the create or change you already made/);
  assert.doesNotMatch(first, /Stop requesting completion/);
  await settle(s);
  await ask('u2');
  assert.match(resultOf(s, 'u2').message, /Stop requesting completion; verify with a read tool or mark the step failed with a note/);
  await settle(s);
  await ask('u3');
  assert.equal(s.state.pending_stop.reason, 'no_progress');
  assert.equal(s.op().kind, 'request_model_turn');
  assert.deepEqual(s.op().tools_offered, []);
});

test('an agent_decides plan with open tasks is nudged', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS, config: { execution_mode: 'agent_decides', approval_optional: true } });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A')]);
  await settle(s);
  await s.modelDone('I will wait.', [], nextOp());
  assert.equal(s.op().kind, 'request_model_turn');
  assert.match(s.op().instructions, /open tasks/);
});

test('a runtime stop finishes with an engine summary of the plan; the model text is not required', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS, config: { no_progress_limit: 1 } });
  await s.start();
  await s.modelDone('', [createPlan('p1', 'A')]);
  await settle(s);
  await s.modelDone('', [{ call_id: 'w', name: 'write', args: { v: 1 } }], nextOp());
  await s.send('operation_failed', { error_class: 'tool_error', message: 'boom', call_id: 'w' }, { effect_id: s.op().effect_id, operation_id: nextOp() });
  await settle(s);
  assert.equal(s.op().kind, 'request_model_turn');
  await s.modelDone('', [], nextOp());
  const f = s.state.finish;
  assert.equal(f.reason, 'no_progress');
  assert.equal(f.status_statement, 'I stopped before finishing. Done: 0 of 2 steps. Still open: one, two. Reason: my last steps did not work. You can ask me to continue.');
  assert.deepEqual(f.tasks_open, [{ key: 't1', title: 'one' }, { key: 't2', title: 'two' }]);
  assert.deepEqual(f.tasks_done, []);
});

test('a failed model turn gives the person a short reason, never the provider message', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS, config: { model_retry_limit: 0 } });
  await s.start();
  const eff = s.op();
  await s.send('operation_failed', { error_class: 'provider', retryable: false, message: 'upstream 500: secret-provider-detail' }, { effect_id: eff.effect_id, operation_id: nextOp() });
  assert.doesNotMatch(s.state.finish.status_statement, /secret-provider-detail/);
  assert.match(s.state.finish.status_statement, /the assistant could not continue/);
  assert.match(s.state.finish.failed.message, /secret-provider-detail/, 'kept for diagnostics');
});

test('a user message resets the loop count', async () => {
  const s = sim({ text: 'x', tools: PLAN_TOOLS });
  await s.start();
  s.state.progress.loop_hits = 2;
  await s.send('user_input', { steer_id: 'run-1/steer/1', input: { text: 'again' } });
  assert.equal(s.state.progress.loop_hits, 0);
});
