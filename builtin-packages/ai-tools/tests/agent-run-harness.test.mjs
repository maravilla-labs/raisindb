/**
 * The ai-tools half of the AgentRun contract: tool-result envelopes and error
 * classes, the model-turn function (context, budget, bounded retries,
 * transcript projection, replay guard), the projection operation, run entry
 * (create vs steer), and user controls routed to the runtime.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-run-harness.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { fakeRaisin } from './support/fake-raisin.mjs';
import { runEnvelope, classifyError, statusForClass, buildEnvelope } from '../content/functions/lib/raisin/ai/agent-shared/tool-envelope.js';
import { runModelTurn, extractCalls } from '../content/functions/lib/raisin/ai/agent-run-model-turn/provider.js';
import { applyBudget, renderFacts, restateInstructions } from '../content/functions/lib/raisin/ai/agent-run-model-turn/context.js';
import { handler as modelTurn } from '../content/functions/lib/raisin/ai/agent-run-model-turn/index.js';
import { handler as project } from '../content/functions/lib/raisin/ai/agent-run-project/index.js';
import { handler as control } from '../content/functions/lib/raisin/ai/agent-run-control/index.js';
import { handler as remember } from '../content/functions/lib/raisin/ai/remember/index.js';
import { handler as readUserContext } from '../content/functions/lib/raisin/ai/read-user-context/index.js';
import { handler as forget } from '../content/functions/lib/raisin/ai/forget/index.js';
import { routeToAgentRun } from '../content/functions/lib/raisin/ai/agent-shared/run-entry.js';
import { turnMessageName } from '../content/functions/lib/raisin/ai/agent-shared/run-names.js';

const CHAT = '/agents/helper/inbox/chats/c1';
const RUN = 'run-1abcdef';

function seedChat(f, extra = {}) {
  f.put('ai', '/agents/helper', { node_type: 'raisin:Folder', properties: { user_id: 'agent:helper', display_name: 'Helper' } });
  f.put('ai', CHAT, { node_type: 'raisin:Conversation', properties: { agent_ref: '/agents/helper', stream_channel: 'chat:c1', ...extra } });
  f.put('functions', '/agents/helper', { node_type: 'raisin:AIAgent', properties: { provider: 'p', model: 'm', system_prompt: 'You help.', tools: ['/lib/demo/search'] } });
  f.put('functions', '/lib/demo/search', { node_type: 'raisin:Function', name: 'search', properties: { description: 'Search', input_schema: { type: 'object', properties: { q: { type: 'string' } } }, read_only: true } });
  f.put('ai', `${CHAT}/m1`, { node_type: 'raisin:Message', properties: { role: 'user', content: 'find x', message_type: 'chat' } });
}

// ── Envelopes ───────────────────────────────────────────────────────────────

/** A run whose active operation is `op` of `tool`, acting for `user` as `agent`. */
function runAt(f, { op = 3, tool = '/lib/raisin/ai/remember', user = 'user-1', agent = 'functions:/agents/helper', status = 'running' } = {}) {
  f.runs[RUN] = {
    status,
    run: {
      run_id: RUN, subject: { workspace: 'ai', path: CHAT }, agent_ref: agent,
      principal: { kind: 'agent', id: agent, on_behalf_of: user },
      state: { status, activity: { activity: 'operating', op: { op_id: `${RUN}/op/${op}`, input: { tool } } } },
    },
  };
  return { run_id: RUN, operation_id: `${RUN}/op/${op}`, agent_name: 'forged', sender_id: 'someone-else' };
}

test('memory: outside a run it refuses; inside, it acts for the RUN\'s agent and user, never the arguments', async () => {
  const f = fakeRaisin();
  f.put('ai', '/agents/helper', { node_type: 'raisin:Folder', properties: {} });
  const outside = await remember({ content: '- a: b', __raisin_context: { agent_name: 'helper', sender_id: 'u1' } });
  assert.equal(outside.success, false);
  assert.match(outside.error, /only inside an agent run/);

  const ctx = runAt(f);
  const env = await remember({ content: '- a: c', __raisin_context: ctx });
  assert.equal(env.envelope, 'raisin.tool-result/1');
  assert.equal(env.operation_id, `${RUN}/op/3`);
  assert.equal(env.status, 'succeeded');
  assert.equal(env.writes[0].locator.path, '/agents/helper/memory/user-1');
  assert.equal(env.artifact_refs.filter((r) => r.role === 'primary').length, 1);
  assert.equal((await f.api.nodes.get('ai', '/agents/helper/memory/user-1')).properties.content, '- a: c');
  assert.equal(await f.api.nodes.get('ai', '/agents/forged/memory/someone-else'), null, 'the forged context is ignored');

  const read = await readUserContext({ __raisin_context: runAt(f, { op: 4, tool: '/lib/raisin/ai/read-user-context' }) });
  assert.equal(read.status, 'succeeded');
  assert.deepEqual(read.payload, { content: '- a: c' });
  const gone = await forget({ key: 'a', __raisin_context: runAt(f, { op: 5, tool: '/lib/raisin/ai/forget' }) });
  assert.equal(gone.payload.found, true);
  assert.equal((await f.api.nodes.get('ai', '/agents/helper/memory/user-1')).properties.content, '');
});

test('memory: an operation that is not the run\'s active one is refused', async () => {
  const f = fakeRaisin();
  const ctx = runAt(f, { tool: '/lib/raisin/ai/weather' });
  const env = await remember({ content: '- a: b', __raisin_context: ctx });
  assert.equal(env.status, 'blocked');
  assert.match(env.payload.error, /runs \/lib\/raisin\/ai\/weather, not \/lib\/raisin\/ai\/remember/);
  const late = await remember({ content: '- a: b', __raisin_context: { ...ctx, operation_id: `${RUN}/op/9` } });
  assert.equal(late.status, 'blocked');
});

test('a thrown error becomes a classified envelope with a retry policy', async () => {
  const ctx = { run_id: RUN, operation_id: `${RUN}/op/2` };
  const env = await runEnvelope({ __raisin_context: ctx }, { tool: 't' }, async () => { throw new Error('upstream 503 overloaded'); });
  assert.equal(env.status, 'retryable');
  assert.equal(env.retry_policy.retryable, true);
  assert.equal(env.diagnostics[0].class, 'transient');
  assert.equal(classifyError(new Error('Permission denied for /x')), 'permission_denied');
  assert.equal(statusForClass('permission_denied'), 'blocked');
  assert.equal(classifyError('title is required'), 'invalid_input');
  const failed = await runEnvelope({ __raisin_context: ctx }, { tool: 't' }, async () => ({ success: false, error: 'nope' }));
  assert.equal(failed.status, 'failed');
});

test('a succeeded envelope with writes always names exactly one primary artifact', () => {
  const w = [{ locator: { workspace: 'a', path: '/x' }, action: 'created' }, { locator: { workspace: 'a', path: '/y' }, action: 'created' }];
  const env = buildEnvelope({ operationId: 'o', writes: w, artifactRefs: [
    { kind: 'k', locator: w[0].locator, role: 'primary' }, { kind: 'k', locator: w[1].locator, role: 'primary' },
  ] });
  assert.equal(env.artifact_refs.filter((r) => r.role === 'primary').length, 1);
  const bare = buildEnvelope({ operationId: 'o', writes: w });
  assert.equal(bare.artifact_refs[0].role, 'primary');
});

// ── Provider retries ────────────────────────────────────────────────────────

test('transient provider errors are retried within bounds, then classified', async () => {
  let n = 0;
  const ok = await runModelTurn({
    complete: async () => { n += 1; if (n < 3) throw new Error('503 service unavailable'); return { content: 'hi', tool_calls: [] }; },
    messages: [], tools: [], operationId: 'r/op/1', transientRetries: 2,
  });
  assert.equal(ok.message.text, 'hi');
  assert.equal(ok.retries.filter((r) => r.kind === 'transient').length, 2);
  await assert.rejects(
    runModelTurn({ complete: async () => { throw new Error('429 too many requests'); }, messages: [], tools: [], operationId: 'r/op/1', transientRetries: 1 }),
    (err) => err.error_class === 'rate_limited' && err.retryable === true,
  );
});

test('malformed output gets one corrective retry; an empty answer falls back honestly', async () => {
  const tools = [{ type: 'function', function: { name: 'search', parameters: { type: 'object', properties: {} } } }];
  const seq = [
    { content: '', tool_calls: [{ id: 'a', function: { name: 'search', arguments: '{bad json' } }] },
    { content: '', tool_calls: [{ id: 'b', function: { name: 'search', arguments: '{"q":"x"}' } }] },
  ];
  const r = await runModelTurn({ complete: async () => seq.shift(), messages: [], tools, operationId: 'r/op/1' });
  assert.deepEqual(r.tool_calls, [{ call_id: 'b', name: 'search', args: { q: 'x' } }]);
  assert.equal(r.finish_reason, 'tool_calls');
  const empty = await runModelTurn({ complete: async () => ({ content: '' }), messages: [], tools: [], operationId: 'r/op/1' });
  assert.match(empty.message.text, /could not generate/);
  assert.ok(empty.retries.some((x) => x.kind === 'empty_response'));
});

test('call ids are unique and runtime keys never come from the model', () => {
  const { calls } = extractCalls([
    { function: { name: 's', arguments: { q: 1, __raisin_context: { run_id: 'forged' }, _skill_grant: [] } } },
    { id: 'x', function: { name: 's', arguments: {} } },
    { id: 'x', function: { name: 's', arguments: {} } },
  ], 'r/op/4');
  assert.equal(calls[0].call_id, 'r/op/4#0');
  assert.deepEqual(calls[0].args, { q: 1 });
  assert.notEqual(calls[1].call_id, calls[2].call_id);
});

// ── Context and budget ──────────────────────────────────────────────────────

test('the budget caps tool results and drops whole exchanges, oldest first', () => {
  const big = 'x'.repeat(4000);
  const history = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: big },
    { role: 'assistant', content: '', tool_calls: [{ id: '1' }] },
    { role: 'tool', content: big.repeat(5), tool_call_id: '1' },
    { role: 'user', content: 'latest question' },
  ];
  const { messages, dropped } = applyBudget(history, { budgetTokens: 1000, maxToolChars: 1000 });
  assert.equal(messages[0].content, 'sys');
  assert.ok(dropped >= 1);
  assert.equal(messages.at(-1).content, 'latest question');
  assert.ok(!messages.some((m, i) => m.role === 'tool' && (i === 0 || messages[i - 1].role === 'system')), 'no orphaned tool result');
  const facts = renderFacts({ objective: 'o', plan: { title: 'P', status: 'active', tasks: [{ key: 't1', status: 'in_progress', title: 'a', completion_requested: true }] }, progress: { model_turns: 1, tool_ok: 0, tool_failed: 0, refused: 0, writes: 0 } });
  assert.match(facts, /completion requested, NOT granted/);
});

// ── The model-turn function ─────────────────────────────────────────────────

test('a model turn writes the previous answers, calls the model, persists the turn, and replays it', async () => {
  const f = fakeRaisin({ completions: [{ content: '', finish_reason: 'tool_calls', model: 'm', usage: { prompt_tokens: 10, completion_tokens: 5 }, tool_calls: [{ id: 'c2', function: { name: 'search', arguments: '{"q":"y"}' } }] }] });
  seedChat(f);
  f.runs[RUN] = { status: 'running', run: { run_id: RUN, subject: { workspace: 'ai', path: CHAT }, state: { status: 'running', activity: { activity: 'operating', op: { op_id: `${RUN}/op/3` } } } } };
  // The previous turn (op 1) asked for c1.
  f.put('ai', `${CHAT}/${turnMessageName(RUN, `${RUN}/op/1`)}`, { node_type: 'raisin:Message', properties: { role: 'assistant', content: '', run_id: RUN, run_tool_calls: [{ call_id: 'c1', name: 'search', args: { q: 'x' } }] } });
  const input = {
    run_id: RUN, operation_id: `${RUN}/op/3`, attempt: 1, agent_ref: 'functions:/agents/helper',
    subject: { workspace: 'ai', path: CHAT },
    request: {
      tools_offered: [{ name: 'search', kind: 'function', function_path: '/lib/demo/search', schema: { type: 'object' } }],
      tool_results: [{ call_id: 'c1', synthetic: false, content: { hits: 2 } }],
      context: { facts: { objective: 'find x', last_turn_op: `${RUN}/op/1`, progress: { model_turns: 1, tool_ok: 1, tool_failed: 0, refused: 0, writes: 0 } } },
    },
  };
  const out = await modelTurn(input);
  assert.deepEqual(out.tool_calls, [{ call_id: 'c2', name: 'search', args: { q: 'y' } }]);
  assert.deepEqual(out.usage, { input_tokens: 10, output_tokens: 5 });
  // The real schema was resolved from the function node.
  const sent = f.calls.completions[0];
  assert.deepEqual(sent.tools[0].function.parameters.properties, { q: { type: 'string' } });
  assert.match(sent.messages[0].content, /Run state \(authoritative/);
  // History carries the answered call c1.
  assert.ok(sent.messages.some((m) => m.role === 'tool' && m.tool_call_id === 'c1'));
  const call = await raisin.nodes.get('ai', `${CHAT}/${turnMessageName(RUN, `${RUN}/op/1`)}/tool-call-c1`);
  assert.equal(call.properties.status, 'completed');
  // Replay: same output, no second model call.
  const again = await modelTurn(input);
  assert.deepEqual(again.tool_calls, out.tool_calls);
  assert.equal(f.calls.completions.length, 1);
  assert.ok(f.events.some((e) => e.type === 'conversation:tool_call_started'));
});

test('a model turn refuses an operation that is not the run\'s active one', async () => {
  const f = fakeRaisin();
  seedChat(f);
  f.runs[RUN] = { status: 'running', run: { run_id: RUN, subject: { workspace: 'ai', path: CHAT }, state: { status: 'running', activity: { activity: 'operating', op: { op_id: `${RUN}/op/9` } } } } };
  const out = await modelTurn({ run_id: RUN, operation_id: `${RUN}/op/3`, agent_ref: '/agents/helper', subject: { workspace: 'ai', path: CHAT }, request: {} });
  assert.equal(out.error_class, 'config');
  assert.equal(f.calls.completions.length, 0);
});

// ── Projection ──────────────────────────────────────────────────────────────

test('the final projection delivers the answer with the runtime status, and the plan card', async () => {
  const f = fakeRaisin();
  seedChat(f);
  f.runs[RUN] = { status: 'running', run: { run_id: RUN, subject: { workspace: 'ai', path: CHAT } } };
  f.put('ai', `${CHAT}/${turnMessageName(RUN, `${RUN}/op/1`)}`, { node_type: 'raisin:Message', properties: { role: 'assistant', content: 'Done!', run_id: RUN } });
  const env = await project({
    reason: 'final',
    plan: { no: 1, title: 'P', status: 'active', anchor_op: `${RUN}/op/1`, tasks: [{ key: 't1', title: 'a', status: 'completed' }, { key: 't2', title: 'b', status: 'pending', completion_requested: true }] },
    finish: { outcome: 'partial', status_statement: 'Plan "P": 1/2 task(s) completed with evidence.' },
    last_text: 'Done!',
    __raisin_context: { workspace: 'ai', chat_path: CHAT, run_id: RUN, operation_id: `${RUN}/op/2` },
  });
  assert.equal(env.status, 'succeeded');
  assert.equal(env.artifact_refs.filter((r) => r.role === 'primary').length, 1);
  const card = await raisin.nodes.get('ai', `${CHAT}/${turnMessageName(RUN, `${RUN}/op/1`)}/plan-run-run1abcd-1`);
  assert.equal(card.properties.run_id, RUN);
  assert.equal(card.properties.projection_of_run, true);
  const t2 = await raisin.nodes.get('ai', `${card.path}/task-2`);
  assert.equal(t2.properties.status, 'pending');
  const done = f.events.find((e) => e.type === 'conversation:done');
  assert.match(done.payload.content, /Status \(from the runtime\)/);
  assert.equal(done.payload.runOutcome, 'partial');
});

test('the projection refuses a run that is not about this conversation', async () => {
  const f = fakeRaisin();
  seedChat(f);
  f.runs[RUN] = { status: 'running', run: { run_id: RUN, subject: { workspace: 'ai', path: '/elsewhere' } } };
  const env = await project({ reason: 'final', __raisin_context: { workspace: 'ai', chat_path: CHAT, run_id: RUN, operation_id: `${RUN}/op/2` } });
  assert.notEqual(env.status, 'succeeded');
});

// ── Entry: create vs steer ──────────────────────────────────────────────────

test('a first message creates a run with the agent\'s tools; the next one steers it', async () => {
  const f = fakeRaisin();
  seedChat(f, { human_sender_id: 'user-7' });
  const agentProps = { ...f.nodes.get(`functions\u0000/agents/helper`).properties, task_creation_enabled: true, tools: ['/lib/demo/search', '/lib/raisin/ai/create-plan'] };
  f.put('functions', '/lib/raisin/ai/create-plan', { node_type: 'raisin:Function', name: 'create-plan', properties: { category: 'planning', input_schema: {} } });
  const chat = await raisin.nodes.get('ai', CHAT);
  const msg = await raisin.nodes.get('ai', `${CHAT}/m1`);
  const first = await routeToAgentRun({ workspace: 'ai', chatPath: CHAT, chat, message: msg, agentProps, agentPath: '/agents/helper', agentWorkspace: 'functions', outboxCtx: null, streamChannel: 'chat:c1', hasSkills: false });
  assert.equal(first.mode, 'created');
  const req = f.calls.creates[0];
  assert.equal(req.reducer.function_path, '/lib/raisin/ai/agent-run-reducer');
  assert.equal(req.as_agent, 'functions:/agents/helper');
  assert.equal(req.on_behalf_of, 'user-7');
  assert.deepEqual(req.input.tools.map((t) => [t.name, t.kind]), [['search', 'function'], ['create-plan', 'domain']]);
  assert.equal(req.input.tools[0].replay_safe, true, 'a read-only tool is replay-safe');
  assert.equal((await raisin.nodes.get('ai', CHAT)).properties.active_agent_run_id, first.run_id);

  f.runs[first.run_id].status = 'running';
  const m2 = f.put('ai', `${CHAT}/m2`, { node_type: 'raisin:Message', properties: { role: 'user', content: 'also y' } });
  const second = await routeToAgentRun({ workspace: 'ai', chatPath: CHAT, chat: await raisin.nodes.get('ai', CHAT), message: m2, agentProps, agentPath: '/agents/helper', agentWorkspace: 'functions', outboxCtx: null, streamChannel: 'chat:c1', hasSkills: false });
  assert.equal(second.mode, 'steered');
  assert.equal(f.calls.controls[0].command.command, 'steer');
  assert.equal(f.calls.controls[0].command.input.message_path, `${CHAT}/m2`);
  assert.equal((await raisin.nodes.get('ai', `${CHAT}/m2`)).properties.run_steer_state, 'queued');
  assert.ok(f.events.some((e) => e.type === 'conversation:steer_queued'));
});

// ── Controls ────────────────────────────────────────────────────────────────

test('approve names the open request and the digest; status reports the run', async () => {
  const f = fakeRaisin();
  seedChat(f);
  f.runs[RUN] = { status: 'waiting', projection: { items: [] }, run: { run_id: RUN, subject: { workspace: 'ai', path: CHAT }, steer_queue: [], state: { status: 'waiting', open: [{ request_id: `${RUN}/req/1`, kind: { kind: 'approval', subject_digest: 'abc', summary: 'Plan' } }] } } };
  const r = await control({ action: 'approve', run_id: RUN, control_id: 'k1' });
  assert.equal(r.accepted, true);
  assert.deepEqual(f.calls.controls[0].command, { command: 'approve', request_id: `${RUN}/req/1`, decision: { decision: 'approve' }, subject_digest: 'abc' });
  const s = await control({ action: 'status', run_id: RUN });
  assert.equal(s.status, 'waiting');
  assert.equal(s.open_approval.subject_digest, 'abc');
  f.runs[RUN].reject = 'run_terminal';
  const no = await control({ action: 'stop', run_id: RUN, control_id: 'k2' });
  assert.equal(no.accepted, false);
});

test('stop and plan approval of a run conversation go to the runtime', async () => {
  const { handler: stop } = await import('../content/functions/lib/raisin/ai/request-conversation-stop/index.js');
  const { handlePlanApproval } = await import('../content/functions/lib/raisin/ai/plan-approval-handler/index.js');
  const f = fakeRaisin();
  seedChat(f, { conversation_id: 'c1', active_agent_run_id: RUN });
  f.put('raisin:access_control', '/users/u/inbox/chats/c1', { node_type: 'raisin:Conversation', properties: { conversation_id: 'c1', stream_channel: 'chat:c1' } });
  const msg = `${CHAT}/${turnMessageName(RUN, `${RUN}/op/1`)}`;
  f.put('ai', msg, { node_type: 'raisin:Message', properties: { role: 'assistant', run_id: RUN } });
  const card = f.put('ai', `${msg}/plan-run-run1abcd-1`, { node_type: 'raisin:AIPlan', properties: { title: 'P', status: 'pending_approval', run_id: RUN, projection_of_run: true, approval_digest: 'd1' } });
  f.runs[RUN] = { status: 'waiting', run: { run_id: RUN, subject: { workspace: 'ai', path: CHAT }, state: { status: 'waiting', open: [{ request_id: `${RUN}/req/1`, kind: { kind: 'approval', subject_digest: 'd1' } }] } } };

  const approved = await handlePlanApproval({ action: 'approve', plan_path: card.path });
  assert.equal(approved.success, true);
  assert.deepEqual(f.calls.controls[0].command.subject_digest, 'd1');
  assert.equal(f.calls.controls[0].command.decision.decision, 'approve');

  const stopped = await stop({ conversation_path: '/users/u/inbox/chats/c1', stream_channel: 'chat:c1' });
  assert.equal(stopped.accepted, true);
  assert.equal(stopped.run_id, RUN);
  assert.equal(f.calls.controls[1].command.command, 'stop');
  // Nothing was written onto the chat node as a control request.
  assert.equal((await raisin.nodes.get('ai', CHAT)).properties.agent_control_request, undefined);
});

test('a reducer that asks for it gets its step instructions restated as the latest message', () => {
  const history = [
    { role: 'system', content: 'You are the builder.' },
    { role: 'assistant', content: '', tool_calls: [{ id: 'c1' }] },
    { role: 'tool', tool_call_id: 'c1', content: '{"code":"accepted"}' },
  ];
  const request = { instructions: 'Repair it with revise_artifact.', context: { restate_instructions: true } };
  const out = restateInstructions(history, request);
  assert.equal(out.length, 4);
  assert.equal(out[3].role, 'user');
  assert.match(out[3].content, /not the person/);
  assert.match(out[3].content, /Repair it with revise_artifact\./);
  assert.equal(history.length, 3, 'the history itself is not modified');
  // Opt-in only, and never after the person's own message.
  assert.equal(restateInstructions(history, { instructions: 'x', context: { checkpoint: true } }), history);
  const asked = history.concat({ role: 'user', content: 'Also lower-case it.' });
  assert.equal(restateInstructions(asked, request), asked);
});
