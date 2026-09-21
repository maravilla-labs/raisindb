/**
 * ONE MALFORMED TOOL CALL MUST NOT END THE TURN (baseline gap 11, 2026-09-21).
 *
 * Measured: Studio Builder (groq / gpt-oss-120b) emitted an execute-function
 * call whose arguments were not JSON, the provider refused the completion, and
 * the turn ended on "Error: Backend error: … Failed to parse tool call arguments
 * as JSON" — no retry, and nothing said about where the run stood.
 *
 * Unit tests for agent-shared/completion-retry.js, then both handlers driven end
 * to end over a mocked `globalThis.raisin` (the harness of
 * agent-prompt-absent.test.mjs) with a model that fails once or twice.
 *
 * Run: node --test builtin-packages/ai-tools/tests/completion-retry.test.mjs
 */
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  completeWithToolCallRetry,
  isMalformedToolCallError,
  failedToolName,
  malformedToolCallStopText,
  MALFORMED_TOOL_CALL_CODE,
} from '../content/functions/lib/raisin/ai/agent-shared/completion-retry.js';
import { handleUserMessage } from '../content/functions/lib/raisin/ai/agent-handler/index.js';
import { handleToolResult } from '../content/functions/lib/raisin/ai/agent-continue-handler/index.js';

/** The exact provider refusal from the round 1 B conversation (shortened). */
const GROQ_REFUSAL =
  'Backend error: AI fallback failed: API request failed: Failed to parse tool call arguments as JSON\n' +
  'Failed generation: {"name": "execute-function", "arguments": {"path":"/lib/studio/generated/homepage-title-validator","cases":[{"name":"missing prefix only","input":{"title":"My Site"},"expect":{"reason":"Add';

// ── the helper ──────────────────────────────────────────────────────────────

test('the round 1 B refusal is recognised, and names its tool', () => {
  assert.equal(isMalformedToolCallError(new Error(GROQ_REFUSAL)), true);
  assert.equal(failedToolName(new Error(GROQ_REFUSAL)), 'execute-function');
  assert.equal(isMalformedToolCallError(new Error('rate limit exceeded')), false);
  assert.equal(isMalformedToolCallError(new Error('connection reset')), false);
});

test('one malformed call is retried once, with a correction the model can act on', async () => {
  const seen = [];
  const out = await completeWithToolCallRetry(async (messages) => {
    seen.push(messages);
    if (seen.length === 1) throw new Error(GROQ_REFUSAL);
    return { content: 'ok' };
  }, [{ role: 'user', content: 'go' }]);
  assert.equal(out.retried, true);
  assert.equal(seen.length, 2);
  const correction = seen[1][seen[1].length - 1];
  assert.equal(correction.role, 'system');
  assert.match(correction.content, /execute-function/);
  assert.match(correction.content, /valid JSON/);
  assert.equal(seen[1].length, seen[0].length + 1, 'history is kept; the correction is appended');
});

test('a second malformed call is thrown as malformed_tool_call, after exactly two attempts', async () => {
  let calls = 0;
  await assert.rejects(
    () => completeWithToolCallRetry(async () => { calls++; throw new Error(GROQ_REFUSAL); }, []),
    (err) => err.code === MALFORMED_TOOL_CALL_CODE && err.attempts === 2 && err.tool === 'execute-function',
  );
  assert.equal(calls, 2);
});

test('any other error is not retried', async () => {
  let calls = 0;
  await assert.rejects(
    () => completeWithToolCallRetry(async () => { calls++; throw new Error('rate limit exceeded'); }, []),
    /rate limit/,
  );
  assert.equal(calls, 1);
});

test('the stop text states the server status for a gated agent', async () => {
  globalThis.raisin = {
    sql: {
      async query(sql) {
        if (/raisin:AITask'/.test(sql)) return [{ path: '/c/plan/t1', properties: { title: 'Test the function', status: 'in_progress' } }];
        return [];
      },
    },
  };
  const text = await malformedToolCallStopText('ai', '/c', { finalize_policy: 'require_verified_completion' }, { tool: 'execute-function' });
  assert.match(text, /This turn stopped/);
  assert.match(text, /execute-function/);
  assert.match(text, /Status \(server-verified\): STOPPED with 1 task\(s\) still open — "Test the function" \(in_progress\)/);
  const ungated = await malformedToolCallStopText('ai', '/c', {}, null);
  assert.doesNotMatch(ungated, /server-verified/);
});

// ── both handlers, end to end ───────────────────────────────────────────────

const AGENT_PATH = '/agents/probe';
const CHAT = '/agents/probe/inbox/chats/c1';
const MSG = `${CHAT}/msg-1`;
const REPLY = `${CHAT}/reply-to-msg-1`;
const RESULT = `${REPLY}/aggregated_result`;
const USER = 'user-1';
const TOOL = '/lib/probe/echo';

function store(agentProps) {
  return {
    [`ai:${MSG}`]: { path: MSG, node_type: 'raisin:Message', properties: { role: 'user', message_type: 'chat', status: 'delivered', content: 'test it' } },
    [`ai:${CHAT}`]: {
      path: CHAT,
      node_type: 'raisin:Conversation',
      properties: {
        agent_ref: { 'raisin:path': AGENT_PATH, 'raisin:workspace': 'functions' },
        participants: [USER],
        human_sender_id: USER,
        human_sender_path: `/users/${USER}`,
      },
    },
    [`ai:${AGENT_PATH}`]: { path: AGENT_PATH, node_type: 'raisin:Agent', properties: { user_id: 'agent:probe', display_name: 'Probe' } },
    [`functions:${AGENT_PATH}`]: {
      path: AGENT_PATH,
      node_type: 'raisin:AIAgent',
      properties: {
        provider: 'groq',
        model: 'llama',
        system_prompt: 'You are a probe.',
        tools: [{ 'raisin:ref': 'fn-echo', 'raisin:path': TOOL, 'raisin:workspace': 'functions' }],
        ...agentProps,
      },
    },
    [`functions:${TOOL}`]: {
      id: 'fn-echo',
      name: 'echo',
      path: TOOL,
      node_type: 'raisin:Function',
      properties: { description: 'Echo.', execution_mode: 'inline', input_schema: { type: 'object', properties: { text: { type: 'string' } } } },
    },
  };
}

/** A runtime whose model refuses `failures` times, then (if ever) answers. */
function installRaisin(nodes, { failures }) {
  const calls = [];
  const created = [];
  globalThis.raisin = {
    nodes: {
      async get(ws, path) { return nodes[`${ws}:${path}`] ?? null; },
      async updateProperty() {},
      async update() {},
      async create(ws, parent, body) {
        const node = { path: `${parent}/${body?.name || 'x'}`, node_type: body?.node_type, properties: body?.properties || {} };
        created.push(node);
        return node;
      },
      async getChildren() { return []; },
      beginTransaction() {
        return {
          async create(ws, parent, body) { return globalThis.raisin.nodes.create(ws, parent, body); },
          commit() {},
        };
      },
    },
    sql: {
      async query(sql) {
        if (/raisin:AITask'/.test(sql)) return [{ path: `${CHAT}/plan/t1`, properties: { title: 'Test the function', status: 'in_progress' } }];
        return [];
      },
    },
    events: { async emit() {} },
    ai: {
      async completion(req) {
        calls.push(req);
        if (calls.length <= failures) throw new Error(GROQ_REFUSAL);
        throw new Error('model-answered');
      },
    },
  };
  return { calls, created };
}

const lastMessage = (req) => req.messages[req.messages.length - 1];
const errorMessages = (created) => created.filter((n) => n.node_type === 'raisin:Message' && n.properties.finish_reason === 'error');

test('first turn: a malformed call twice ends the turn honestly, with the server status, and does not throw', async () => {
  const nodes = store({ finalize_policy: 'require_verified_completion' });
  const { calls, created } = installRaisin(nodes, { failures: 2 });
  await handleUserMessage({ workspace: 'ai', event: { node_path: MSG } });
  assert.equal(calls.length, 2, 'exactly one retry');
  assert.equal(lastMessage(calls[1]).role, 'system');
  assert.match(lastMessage(calls[1]).content, /not valid JSON/);
  assert.ok(Array.isArray(calls[1].tools) && calls[1].tools.length > 0, 'the retry still offers the tools');
  const [err] = errorMessages(created);
  assert.ok(err, 'a terminal message was written');
  assert.match(err.properties.content, /This turn stopped/);
  assert.match(err.properties.content, /Status \(server-verified\): STOPPED with 1 task\(s\) still open/);
  assert.doesNotMatch(err.properties.content, /Backend error/);
});

test('first turn: a malformed call once is retried and the turn goes on', async () => {
  const nodes = store({});
  const { calls } = installRaisin(nodes, { failures: 1 });
  // The mock model "answers" by throwing a different error, which is how this
  // harness proves the retry reached the model and the loop carried on.
  await assert.rejects(() => handleUserMessage({ workspace: 'ai', event: { node_path: MSG } }), /model-answered/);
  assert.equal(calls.length, 2);
});

test('continuation: a malformed call twice ends the turn honestly too', async () => {
  const nodes = store({ finalize_policy: 'require_verified_completion' });
  nodes[`ai:${REPLY}`] = { path: REPLY, node_type: 'raisin:Message', properties: { role: 'assistant', dispatch_phase: 'awaiting_results', finish_reason: 'tool_calls' } };
  nodes[`ai:${RESULT}`] = { path: RESULT, node_type: 'raisin:AIToolResultAggregator', properties: { results: [] } };
  const { calls, created } = installRaisin(nodes, { failures: 2 });
  await handleToolResult({ flow_input: { workspace: 'ai', event: { node_path: RESULT } } });
  assert.equal(calls.length, 2, 'exactly one retry');
  assert.match(lastMessage(calls[1]).content, /not valid JSON/);
  const [err] = errorMessages(created);
  assert.ok(err, 'a terminal message was written');
  assert.match(err.properties.content, /This turn stopped/);
  assert.match(err.properties.content, /server-verified/);
});

test('first turn: a completion whose only tool calls were unusable gets one retry, then an honest stop', async () => {
  const nodes = store({ finalize_policy: 'require_verified_completion' });
  const { calls, created } = installRaisin(nodes, { failures: 0 });
  // The model "answers", but every tool call it returns has no name.
  globalThis.raisin.ai.completion = async (req) => {
    calls.push(req);
    return { content: '', tool_calls: [{ id: 'x', function: { arguments: '{"a":' } }], finish_reason: 'tool_calls' };
  };
  try {
    await handleUserMessage({ workspace: 'ai', event: { node_path: MSG } });
  } catch (_) { /* later terminal plumbing is outside this harness; what was persisted is what matters */ }
  assert.equal(calls.length, 2, 'exactly one retry');
  assert.match(lastMessage(calls[1]).content, /not valid JSON/);
  const said = created
    .filter((n) => n.node_type === 'raisin:Message' && n.properties.role === 'assistant')
    .map((n) => String(n.properties.content || (n.properties.body && n.properties.body.content) || ''));
  assert.ok(said.some((t) => /This turn stopped/.test(t) || /Status \(server-verified\)/.test(t)), JSON.stringify(said));
  assert.ok(!said.some((t) => /Please try again\./.test(t)), 'not the old silent "please try again"');
});
