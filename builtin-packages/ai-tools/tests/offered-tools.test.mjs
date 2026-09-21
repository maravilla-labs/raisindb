/**
 * A model's tool call runs ONLY through the tools it was offered this turn.
 *
 * The JS half of the Rust agent-step refusal (`offered_function` /
 * `unoffered_tool_error` / `strip_runtime_keys` in
 * crates/raisin-flow-runtime/src/handlers/ai_tool_loop.rs). In the chat loop
 * "invoking" a tool means queueing a `raisin:AIToolCall` with status `pending`
 * and a `function_ref` — the executor runs exactly that ref. So "never invoked"
 * is asserted as: no pending call node, no function_ref, and the model gets an
 * error result it can read.
 *
 * Run: node --test builtin-packages/ai-tools/tests/offered-tools.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import {
  offeredToolRefs,
  resolveOfferedTool,
  unofferedToolError,
  stripRuntimeArgs,
} from '../content/functions/lib/raisin/ai/agent-shared/tools.js';
import { handleUserMessage } from '../content/functions/lib/raisin/ai/agent-handler/index.js';

const def = (name) => ({ type: 'function', function: { name, description: '', parameters: { type: 'object', properties: {} } } });
const ref = (path) => ({ 'raisin:ref': `id-${path}`, 'raisin:workspace': 'functions', 'raisin:path': path, execution_mode: 'async', category: null });

// ── The pure half ───────────────────────────────────────────────────────────

test('only a tool in the offered definitions resolves', () => {
  const given = { 'get-weather': ref('/lib/weather'), 'update-node': ref('/lib/update-node') };
  const offered = offeredToolRefs([def('get-weather')], given);
  assert.deepEqual(resolveOfferedTool(offered, 'get-weather'), given['get-weather']);
  // GIVEN to the agent but withdrawn this turn (the loop guard's case): refused.
  assert.equal(resolveOfferedTool(offered, 'update-node'), null);
  // A function PATH is not a tool name.
  assert.equal(resolveOfferedTool(offered, '/lib/update-node'), null);
});

test('prototype member names never resolve', () => {
  const given = { 'get-weather': ref('/lib/weather') };
  const offered = offeredToolRefs([def('get-weather'), def('constructor'), def('__proto__')], given);
  for (const name of ['constructor', 'toString', '__proto__', 'hasOwnProperty']) {
    assert.equal(resolveOfferedTool(offered, name), null, name);
  }
});

test('no offer means nothing resolves', () => {
  const given = { 'get-weather': ref('/lib/weather') };
  const offered = offeredToolRefs([], given);
  assert.equal(resolveOfferedTool(offered, 'get-weather'), null);
  assert.equal(unofferedToolError('get-weather', offered),
    '`get-weather` is not a tool offered to you, so it was not run. No tools are offered in this step.');
});

test('the refusal names what IS offered, sorted — the Rust wording', () => {
  const given = { b: ref('/b'), a: ref('/a') };
  const offered = offeredToolRefs([def('b'), def('a')], given);
  assert.equal(unofferedToolError('/lib/studio/anything', offered),
    '`/lib/studio/anything` is not a tool offered to you, so it was not run. The tools offered to you are: a, b.');
});

test('runtime-only keys are stripped from model arguments; the rest survives', () => {
  const out = stripRuntimeArgs({
    city: 'Bern',
    __raisin_flow: { instance_id: 'someone-elses-flow', step_id: 'approve' },
    _skill_grant: [{ name: 'root', workspace: 'functions', path: '/skills/root' }],
    __raisin_context: { chat_path: '/x' },
  });
  assert.deepEqual(out, { city: 'Bern', __raisin_context: { chat_path: '/x' } });
  assert.equal(stripRuntimeArgs(null), null);
  assert.deepEqual(stripRuntimeArgs([1]), [1]);
});

// ── The handler, end to end over an in-memory raisin ───────────────────────

const WS = 'ai';
const AGENT = '/agents/weather-bot';
const CHAT = `${AGENT}/inbox/chats/chat-1`;
const MSG = `${CHAT}/msg-1`;

function fakeRaisin(toolCalls) {
  const nodes = new Map();
  const completions = [];
  const put = (path, node) => nodes.set(path, { path, name: path.split('/').pop(), ...node });
  put(MSG, { node_type: 'raisin:Message', properties: { role: 'user', message_type: 'chat', status: 'delivered', content: 'weather in Bern?' } });
  put(CHAT, { node_type: 'raisin:Conversation', properties: { agent_ref: AGENT, participants: ['user-1'], human_sender_id: 'user-1', human_sender_path: '/users/user-1' } });
  put(AGENT, { node_type: 'raisin:AIAgent', properties: { provider: 'test', model: 'm', tools: ['/lib/weather'] } });
  put('/lib/weather', { id: 'fn-weather', node_type: 'raisin:Function', name: 'get-weather', properties: { description: 'Weather', input_schema: { type: 'object', properties: { city: { type: 'string' } } } } });
  // Exists as a function but is NOT in the agent's tools:.
  put('/lib/studio/delete-everything', { id: 'fn-danger', node_type: 'raisin:Function', name: 'delete-everything', properties: {} });

  const noop = async () => null;
  const nodesApi = {
    async get(_ws, path) { return nodes.get(path) || null; },
    async create(_ws, parent, body) {
      const path = `${parent}/${body.name}`;
      if (nodes.has(path)) return { error: 'already exists' };
      put(path, { ...body, properties: { ...(body.properties || {}) } });
      return nodes.get(path);
    },
    async update(_ws, path, body) {
      const n = nodes.get(path);
      if (n) n.properties = { ...n.properties, ...(body?.properties || {}) };
      return n;
    },
    async updateProperty(_ws, path, key, value) {
      const n = nodes.get(path);
      if (n) n.properties = { ...n.properties, [key]: value };
    },
    async getChildren(_ws, parent) {
      return [...nodes.values()].filter((n) => n.path.slice(0, n.path.lastIndexOf('/')) === parent);
    },
    beginTransaction() {
      return { create: (...a) => nodesApi.create(...a), commit() {}, rollback() {} };
    },
  };
  const api = {
    nodes: new Proxy(nodesApi, { get: (t, k) => t[k] || noop }),
    sql: { async query() { return []; }, async execute() { return 0; } },
    events: { async emit() {} },
    ai: {
      async completion(req) {
        completions.push(req);
        return { content: '', finish_reason: 'tool_calls', model: 'm', tool_calls: toolCalls };
      },
    },
  };
  globalThis.raisin = new Proxy(api, { get: (t, k) => t[k] || new Proxy({}, { get: () => noop }) });
  return { nodes, completions };
}

const call = (id, name, args) => ({ id, type: 'function', function: { name, arguments: JSON.stringify(args) } });
const callNodes = (nodes) => [...nodes.values()].filter((n) => n.node_type === 'raisin:AIToolCall');

test('handler: an unoffered function name is refused and never queued; an offered one is', async () => {
  const { nodes, completions } = fakeRaisin([
    call('c1', 'get-weather', { city: 'Bern' }),
    call('c2', 'delete-everything', { all: true }),
    call('c3', '/lib/studio/delete-everything', { all: true }),
    call('c4', 'constructor', {}),
  ]);

  await handleUserMessage({ workspace: WS, event: { node_path: MSG } });

  assert.equal(completions.length >= 1, true, 'the model was called');
  assert.deepEqual(completions[0].tools.map((t) => t.function.name), ['get-weather'], 'the offer');

  const calls = callNodes(nodes);
  const byId = Object.fromEntries(calls.map((c) => [c.properties.tool_call_id, c]));

  // Offered → queued for execution with its own ref.
  assert.equal(byId.c1.properties.status, 'pending');
  assert.equal(byId.c1.properties.function_ref['raisin:path'], '/lib/weather');
  assert.equal(byId.c1.properties.arguments.city, 'Bern');

  // Not offered → a completed call with an error result, and NO function_ref.
  for (const [id, name] of [['c2', 'delete-everything'], ['c3', '/lib/studio/delete-everything'], ['c4', 'constructor']]) {
    const c = byId[id];
    assert.ok(c, `a call node for ${id}`);
    assert.equal(c.properties.status, 'completed', id);
    assert.equal(c.properties.function_ref, undefined, `${id} must carry no function_ref`);
    const result = nodes.get(`${c.path}/result`);
    assert.equal(result.properties.result.error,
      `\`${name}\` is not a tool offered to you, so it was not run. The tools offered to you are: get-weather.`);
  }
  assert.equal(calls.filter((c) => c.properties.status === 'pending').length, 1, 'only the offered call is pending');
});

test('handler: a model-supplied __raisin_flow / _skill_grant never reaches the function', async () => {
  const { nodes } = fakeRaisin([
    call('c1', 'get-weather', {
      city: 'Bern',
      __raisin_flow: { instance_id: 'someone-elses-flow', step_id: 'approve' },
      _skill_grant: [{ name: 'root', workspace: 'functions', path: '/skills/root' }],
    }),
  ]);

  await handleUserMessage({ workspace: WS, event: { node_path: MSG } });

  const [c] = callNodes(nodes);
  assert.equal(c.properties.status, 'pending');
  assert.equal('__raisin_flow' in c.properties.arguments, false);
  assert.equal('_skill_grant' in c.properties.arguments, false);
  assert.equal(c.properties.arguments.city, 'Bern');
  // The existing __raisin_context stamp is still written by the handler.
  assert.equal(c.properties.arguments.__raisin_context.chat_path, CHAT);
});

// ── The continuation: GIVEN is not OFFERED ─────────────────────────────────

/**
 * The continue handler resolved calls through `toolNameToRef` — every tool the
 * agent was GIVEN — so a turn that offered the model nothing (a completed plan
 * forces a final text answer; the loop guard withdraws a repeated tool) still
 * queued whatever the model named. A completed plan is the simplest way to get
 * an empty offer without a history fixture.
 */
test('continue: a given-but-not-offered tool is refused on a no-tools turn', async () => {
  const REPLY = `${CHAT}/reply-to-msg-1`;
  const { nodes, completions } = fakeRaisin([call('c9', 'get-weather', { city: 'Bern' })]);
  nodes.set(REPLY, { path: REPLY, name: 'reply-to-msg-1', node_type: 'raisin:Message', properties: { role: 'assistant', finish_reason: 'tool_calls' } });
  nodes.set(`${REPLY}/aggregated_result`, {
    path: `${REPLY}/aggregated_result`,
    name: 'aggregated_result',
    node_type: 'raisin:AIToolResult',
    properties: { results: [{ function_name: 'get-plan-status', result: { status: 'completed', total_tasks: 1, completed_tasks: 1, pending_tasks: 0 } }] },
  });

  const { handleToolResult } = await import('../content/functions/lib/raisin/ai/agent-continue-handler/index.js');
  await handleToolResult({ flow_input: { workspace: WS, event: { node_path: `${REPLY}/aggregated_result` } } });

  assert.ok(completions.length >= 1, 'the model was called');
  assert.equal(completions[0].tools, undefined, 'nothing was offered');
  const calls = callNodes(nodes);
  assert.equal(calls.length, 1);
  const [c] = calls;
  assert.equal(c.properties.status, 'completed');
  assert.equal(c.properties.function_ref, undefined, 'never queued with a ref');
  assert.equal(nodes.get(`${c.path}/result`).properties.result.error,
    '`get-weather` is not a tool offered to you, so it was not run. No tools are offered in this step.');
});

test('continue: an offered tool still queues, with runtime-only keys stripped', async () => {
  const REPLY = `${CHAT}/reply-to-msg-1`;
  const { nodes, completions } = fakeRaisin([
    call('c10', 'get-weather', { city: 'Bern', __raisin_flow: { instance_id: 'x', step_id: 'y' }, _skill_grant: [] }),
  ]);
  nodes.set(REPLY, { path: REPLY, name: 'reply-to-msg-1', node_type: 'raisin:Message', properties: { role: 'assistant', finish_reason: 'tool_calls' } });
  nodes.set(`${REPLY}/aggregated_result`, {
    path: `${REPLY}/aggregated_result`,
    name: 'aggregated_result',
    node_type: 'raisin:AIToolResult',
    properties: { results: [{ function_name: 'get-weather', result: { temp: 12 } }] },
  });

  const { handleToolResult } = await import('../content/functions/lib/raisin/ai/agent-continue-handler/index.js');
  await handleToolResult({ flow_input: { workspace: WS, event: { node_path: `${REPLY}/aggregated_result` } } });

  assert.deepEqual(completions[0].tools.map((t) => t.function.name), ['get-weather']);
  const [c] = callNodes(nodes);
  assert.equal(c.properties.status, 'pending');
  assert.equal(c.properties.function_ref['raisin:path'], '/lib/weather');
  assert.deepEqual(Object.keys(c.properties.arguments).sort(), ['__raisin_context', 'city']);
  assert.equal(c.properties.arguments.__raisin_context.chat_path, CHAT);
});

/**
 * The "repeated unknown tool" give-up is for a model that keeps naming a tool
 * that was REFUSED — not for one that calls, on a no-tools turn, a tool that
 * really ran last round. Checking the previous round's calls against THIS
 * turn's offer counted every tool that ran before as "unknown", so the most
 * likely call on a forced-final or loop-guard turn ended the whole turn with
 * "doesn't exist" instead of the refusal the model could read and recover from.
 */
test('continue: re-calling a tool that ran last round is refused, not a terminal "unknown tool" error', async () => {
  const REPLY = `${CHAT}/reply-to-msg-1`;
  const { nodes } = fakeRaisin([call('c12', 'get-weather', { city: 'Bern' })]);
  nodes.set(REPLY, { path: REPLY, name: 'reply-to-msg-1', node_type: 'raisin:Message', properties: { role: 'assistant', finish_reason: 'tool_calls' } });
  // Last round's call: offered, queued with its ref, executed.
  nodes.set(`${REPLY}/tool-call-c11`, {
    path: `${REPLY}/tool-call-c11`,
    name: 'tool-call-c11',
    node_type: 'raisin:AIToolCall',
    properties: { tool_call_id: 'c11', function_name: 'get-weather', function_ref: ref('/lib/weather'), arguments: { city: 'Bern' }, status: 'completed' },
  });
  nodes.set(`${REPLY}/aggregated_result`, {
    path: `${REPLY}/aggregated_result`,
    name: 'aggregated_result',
    node_type: 'raisin:AIToolResult',
    properties: { results: [{ function_name: 'get-plan-status', result: { status: 'completed', total_tasks: 1, completed_tasks: 1, pending_tasks: 0 } }] },
  });

  const { handleToolResult } = await import('../content/functions/lib/raisin/ai/agent-continue-handler/index.js');
  await handleToolResult({ flow_input: { workspace: WS, event: { node_path: `${REPLY}/aggregated_result` } } });

  const c = [...nodes.values()].find((n) => n.node_type === 'raisin:AIToolCall' && n.properties.tool_call_id === 'c12');
  assert.ok(c, 'the refused call is recorded for the model to read');
  assert.equal(c.properties.status, 'completed');
  assert.equal(c.properties.function_ref, undefined, 'never queued with a ref');
  assert.equal(nodes.get(`${c.path}/result`).properties.result.error,
    '`get-weather` is not a tool offered to you, so it was not run. No tools are offered in this step.');
});

test('continue: a SECOND refusal in a row still gives up', async () => {
  const REPLY = `${CHAT}/reply-to-msg-1`;
  const { nodes } = fakeRaisin([call('c14', 'delete-everything', {})]);
  nodes.set(REPLY, { path: REPLY, name: 'reply-to-msg-1', node_type: 'raisin:Message', properties: { role: 'assistant', finish_reason: 'tool_calls' } });
  // Last round's call was itself refused: completed, no function_ref.
  nodes.set(`${REPLY}/tool-call-c13`, {
    path: `${REPLY}/tool-call-c13`,
    name: 'tool-call-c13',
    node_type: 'raisin:AIToolCall',
    properties: { tool_call_id: 'c13', function_name: 'delete-everything', arguments: {}, status: 'completed' },
  });
  nodes.set(`${REPLY}/aggregated_result`, {
    path: `${REPLY}/aggregated_result`,
    name: 'aggregated_result',
    node_type: 'raisin:AIToolResult',
    properties: { results: [{ function_name: 'delete-everything', result: { error: 'not offered' } }] },
  });

  const { handleToolResult } = await import('../content/functions/lib/raisin/ai/agent-continue-handler/index.js');
  await handleToolResult({ flow_input: { workspace: WS, event: { node_path: `${REPLY}/aggregated_result` } } });

  const calls = [...nodes.values()].filter((n) => n.node_type === 'raisin:AIToolCall');
  assert.equal(calls.some((n) => n.properties.tool_call_id === 'c14'), false, 'gave up: no second call node');
  assert.equal(calls.some((n) => n.properties.status === 'pending'), false, 'nothing queued');
});
