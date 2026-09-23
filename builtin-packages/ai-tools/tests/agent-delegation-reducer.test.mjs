/**
 * Child runs inside the generic reducer: children learned from tool results
 * and from core's hand-backs, shown as facts; a hand-back into a wait that
 * re-issues the wait until every named child is done; a user steer
 * interrupting a wait (the children keep working); a child's own input and a
 * parent's message unwrapped from core's shapes; and a delegated run's write
 * grant (refused before, detected after). Stopping live children when a run
 * ends is core's cascade — the reducer issues none.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-delegation-reducer.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { handler } from '../content/functions/lib/raisin/ai/agent-run-reducer/index.js';
import { PROJECT_FUNCTION } from '../content/functions/lib/raisin/ai/agent-shared/run-names.js';

const OPS = new Set(['call_tool', 'request_model_turn', 'request_approval', 'ask_user']);
const SPAWN = '/lib/raisin/ai/spawn-agent';
const WAIT = '/lib/raisin/ai/wait-for-agents';

function sim(input) {
  const s = { seq: 0, rev: 0, state: null, open: [], status: 'running' };
  s.send = async (kind, data = {}, extra = {}) => {
    s.seq += 1;
    const req = {
      contract: 'raisin.agent-run.reducer/1',
      run: { run_id: 'run-1', status: s.status, open_requests: s.open, unanswered_calls: [], subject: { workspace: 'ai', path: '/agents/a/inbox/chats/c' } },
      state: s.state, state_rev: s.rev, event: { seq: s.seq, kind, data, ...extra },
    };
    const resp = await handler(req);
    assert.ok(resp.effects.filter((e) => OPS.has(e.kind)).length <= 1, 'one operation');
    s.state = resp.state;
    s.rev = resp.state_rev;
    s.last = resp;
    return resp;
  };
  s.op = () => s.last.effects.find((e) => OPS.has(e.kind));
  s.model = (text, calls = []) => {
    const eff = s.op();
    assert.equal(eff.kind, 'request_model_turn');
    return s.send('model_turn_completed', { message: { text }, tool_calls: calls }, { effect_id: eff.effect_id, operation_id: `run-1/op/${s.seq}` });
  };
  s.tool = (envelope) => {
    const eff = s.op();
    assert.equal(eff.kind, 'call_tool');
    return s.send('tool_result', { call_id: eff.for_call_id, tool: eff.tool, envelope }, { effect_id: eff.effect_id, operation_id: `run-1/op/${s.seq}` });
  };
  s.start = () => s.send('run_started', { input });
  return s;
}

const env = (payload, extra = {}) => ({ envelope: 'raisin.tool-result/1', operation_id: 'x', status: 'succeeded', writes: [], artifact_refs: [], payload, ...extra });
const TOOLS = [
  { name: 'spawn_agent', function_path: SPAWN, mutating: true, replay_safe: true },
  { name: 'wait_for_agents', function_path: WAIT, mutating: true, replay_safe: true },
  { name: 'write_node', function_path: '/lib/demo/write', mutating: true },
];

test('children are tracked and shown; a steer interrupts the wait; finishing issues no cascade (core stops them)', async () => {
  const s = sim({ text: 'do it', tools: TOOLS, context: { workspace: 'ai', chat_path: '/agents/a/inbox/chats/c' } });
  await s.start();
  await s.model('', [{ call_id: 'c1', name: 'spawn_agent', args: { objective: 'x', key: 'rev' } }]);
  await s.tool(env({ child: { run_id: 'run-c1', key: 'rev', agent_ref: '/agents/r', status: 'queued' } }));
  const facts = s.op().context.facts;
  assert.deepEqual(facts.children.map((c) => [c.key, c.status]), [['rev', 'queued']]);

  // The model waits; core parks the call as an external request.
  await s.model('', [{ call_id: 'c2', name: 'wait_for_agents', args: {} }]);
  const waitEff = s.op();
  assert.equal(waitEff.tool, WAIT);
  s.open = [{ request_id: 'run-1/req/1', kind: 'external', effect_id: waitEff.effect_id }];

  // The user writes meanwhile: the wait is withdrawn and answered, the model is asked.
  await s.send('user_input', { input: { text: 'also check the FAQ' } });
  assert.ok(s.last.effects.some((e) => e.kind === 'withdraw_request' && e.request_id === 'run-1/req/1'));
  const turn = s.op();
  assert.equal(turn.kind, 'request_model_turn');
  const answered = turn.tool_results.find((r) => r.call_id === 'c2');
  assert.equal(answered.content.status, 'interrupted');
  s.open = [];

  // The model finishes while the child still runs: straight to the final
  // projection; the runtime stops the child when this run ends.
  await s.model('All done.');
  const final = s.op();
  assert.equal(final.tool, PROJECT_FUNCTION);
  assert.equal(final.args.reason, 'final');
  await s.tool(env({ reason: 'final' }));
  assert.ok(s.last.effects.some((e) => e.kind === 'complete'));
});

test('a delivered hand-back marks children done, so finishing needs no cascade', async () => {
  const s = sim({ text: 'do it', tools: TOOLS });
  await s.start();
  await s.model('', [{ call_id: 'c1', name: 'spawn_agent', args: { objective: 'x' } }]);
  await s.tool(env({ child: { run_id: 'run-c1', key: 'a', status: 'queued' } }));
  await s.model('', [{ call_id: 'c2', name: 'wait_for_agents', args: {} }]);
  await s.tool(env({ done: true, children: [{ run_id: 'run-c1', key: 'a', status: 'completed', outcome: 'succeeded', accepted: true, handed_back: true }] }));
  assert.equal(s.op().context.facts.children[0].accepted, true);
  await s.model('Finished.');
  assert.equal(s.op().tool, PROJECT_FUNCTION);
});

test('a delegated run\'s write grant: out-of-scope calls are refused, reported strays block the run', async () => {
  const s = sim({
    text: 'fix typos', tools: TOOLS,
    config: { write_scope: [{ workspace: 'content', path: '/docs' }] },
    context: { workspace: 'ai', chat_path: '/agents/r/inbox/chats/deleg-x', delegation: { parent_run_id: 'run-p', key: 'rev', depth: 1 } },
  });
  await s.start();
  await s.model('', [{ call_id: 'c1', name: 'write_node', args: { workspace: 'content', path: '/blog/x' } }]);
  const turn = s.op();
  assert.equal(turn.kind, 'request_model_turn');
  assert.equal(turn.tool_results[0].content.status, 'refused');
  assert.match(turn.tool_results[0].content.error, /outside what this delegated run may write/);

  await s.model('', [{ call_id: 'c2', name: 'write_node', args: { workspace: 'content', path: '/docs/a' } }]);
  assert.equal(s.op().kind, 'call_tool');
  await s.tool(env({ ok: true }, { writes: [{ locator: { workspace: 'content', path: '/blog/other' }, action: 'updated' }] }));
  const final = s.op();
  assert.equal(final.tool, PROJECT_FUNCTION);
  assert.equal(s.state.finish.outcome, 'blocked');
  assert.equal(s.state.finish.reason, 'write_scope_violation');
  assert.equal(final.args.__raisin_context.delegation.parent_run_id, 'run-p');
  // The hand-back is core's, from the terminal outcome: artifacts carry the
  // `kind` + `locator` core matches expected artifacts on.
  await s.tool(env({ reason: 'final' }));
  const done = s.last.effects.find((e) => e.kind === 'complete');
  assert.equal(done.outcome, 'blocked');
  assert.deepEqual(done.artifacts.map((a) => [a.kind, a.locator.path]), [['node', '/blog/other']]);
});

const handback = (childRunId, status = 'completed', kind = 'succeeded') => ({
  envelope: 'raisin.tool-result/1', operation_id: 'run-1/op/9', status: status === 'completed' ? 'succeeded' : 'failed',
  diagnostics: [], payload: { child_run_id: childRunId, child_no: 1, title: 'x', status, outcome: { kind }, usage: {}, contract: { satisfied: true, violations: [] } },
});

test('a hand-back into a wait re-issues the wait for the same call, until it answers for every child', async () => {
  const s = sim({ text: 'do it', tools: TOOLS });
  await s.start();
  await s.model('', [
    { call_id: 'c1', name: 'spawn_agent', args: { objective: 'a', key: 'a' } },
  ]);
  await s.tool(env({ child: { run_id: 'run-a', key: 'a', status: 'queued' } }));
  await s.model('', [{ call_id: 'c2', name: 'wait_for_agents', args: { agents: ['a', 'b'] } }]);
  const first = s.op();
  // Core delivers child a's hand-back into the waiting call.
  await s.tool(handback('run-a'));
  const again = s.op();
  assert.equal(again.kind, 'call_tool', 'the wait is issued again, not answered with one child');
  assert.equal(again.tool, WAIT);
  assert.equal(again.for_call_id, 'c2');
  assert.notEqual(again.effect_id, first.effect_id);
  assert.deepEqual(again.args.agents, ['a', 'b']);
  assert.equal(s.state.children[0].handed_back, true);
  // A failed child's hand-back is a hand-back too.
  await s.tool(handback('run-b', 'failed', 'failed'));
  assert.equal(s.op().tool, WAIT);
  // The wait answers for all of them: that is the model's one result.
  await s.tool(env({ done: true, mode: 'all', children: [{ run_id: 'run-a', status: 'completed' }, { run_id: 'run-b', status: 'failed' }] }));
  const turn = s.op();
  assert.equal(turn.kind, 'request_model_turn');
  const r = turn.tool_results.find((x) => x.call_id === 'c2');
  assert.equal(r.content.result.done, true);
  assert.equal(s.state.progress.tool_calls, 2, 're-issued waits are not new model calls');
});

test('a child starts from its own input inside core\'s delegation envelope; a parent\'s message is its steer', async () => {
  const s = sim({
    objective: { title: 'x' }, context: { mode: 'none' }, parent_run_id: 'run-p', child_no: 1,
    input: { text: 'the brief', tools: TOOLS, config: { write_scope: [] }, context: { chat_path: '/agents/r/inbox/chats/d', delegation: { parent_run_id: 'run-p' } } },
  });
  await s.start();
  assert.equal(s.state.objective, 'the brief');
  assert.deepEqual(s.state.cfg.write_scope, []);
  assert.equal(s.state.tools.length, 3);
  await s.send('user_input', { input: { type: 'parent_message', from_run: 'run-p', message: { text: 'use the FAQ' } } });
  assert.deepEqual(s.state.steer_texts, ['use the FAQ'], 'no transcript path: the text reaches the next turn');
  await s.send('user_input', { input: { type: 'parent_steer', from_run: 'run-p', input: { text: 'stop at 3', message_path: '/agents/r/inbox/chats/d/parent-x' } } });
  assert.deepEqual(s.state.steer_texts, ['use the FAQ'], 'a transcript message is read from the transcript, not repeated');
});

test('a read-only delegated run may not call a mutating tool at all', async () => {
  const s = sim({ text: 'look', tools: TOOLS, config: { write_scope: [] } });
  await s.start();
  await s.model('', [{ call_id: 'c1', name: 'write_node', args: {} }]);
  assert.match(s.op().tool_results[0].content.error, /read-only/);
});
