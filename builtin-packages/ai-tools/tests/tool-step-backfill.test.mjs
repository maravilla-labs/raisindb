// TOOL STEPS OF THE PERSON'S HISTORY: names, hard stops, the Stop button,
// ordering and the admin backfill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from './support/fake-raisin.mjs';

fakeRaisin();
const { toolCallMirrorSlug } = await import('../content/functions/lib/raisin/ai/agent-shared/run-names.js');
const { sendAgentOutboxMessage } = await import('../content/functions/lib/raisin/ai/agent-shared/outbox.js');
const { handler: project } = await import('../content/functions/lib/raisin/ai/agent-run-project/index.js');
const { handler: backfill, chatPathOfCall } = await import('../content/functions/lib/raisin/ai/backfill-tool-steps/index.js');
const { handler: stop } = await import('../content/functions/lib/raisin/ai/request-conversation-stop/index.js');
const { pendingToolResults } = await import('../content/functions/lib/raisin/ai/agent-run-reducer/steps.js');

const AGENT = '/agents/site';
const CHAT = `${AGENT}/inbox/chats/chat-1`;
const RUN = 'abcd1234-aaaa-bbbb-cccc-000000000001';
const outbox = { agentHomePath: AGENT, agentUserId: 'agent:site', agentDisplayName: 'Site', senderId: 'u1', senderPath: '/users/internal/u1', conversationId: 'chat-1' };
const steps = (h) => h.calls.outbox.filter((b) => b.properties.message_type === 'ai_tool_call');

function world({ calls = [], stored = [] } = {}) {
  const h = fakeRaisin();
  h.put('ai', AGENT, { node_type: 'raisin:Folder', properties: { user_id: 'agent:site', display_name: 'Site' } });
  h.put('ai', CHAT, { node_type: 'raisin:Conversation', properties: { human_sender_id: 'u1', human_sender_path: '/users/internal/u1', participants: ['u1', 'agent:site'] } });
  const turn = h.put('ai', `${CHAT}/run-abcd1234-op-1`, { node_type: 'raisin:Message', properties: { role: 'assistant', content: 'x', run_id: RUN, run_tool_calls: calls } });
  h.runs[RUN] = { status: 'running', run: { run_id: RUN, subject: { workspace: 'ai', path: CHAT } } };
  for (const [i, s] of stored.entries()) {
    const call = h.put('ai', `${CHAT}/run-abcd1234-op-${i + 2}/tool-call-${s.id}`, {
      node_type: 'raisin:AIToolCall', created_at: s.at,
      properties: { tool_call_id: s.id, function_name: s.name, arguments: s.args || {}, status: s.status || 'completed', run_id: RUN },
    });
    h.put('ai', `${call.path}/result`, { node_type: 'raisin:AIToolResult', properties: { result: s.result || { ok: true } } });
  }
  return { h, turn };
}

test('the mirror name is deterministic and collision free across many ids', () => {
  assert.equal(toolCallMirrorSlug(RUN, 'call_1'), toolCallMirrorSlug(RUN, 'call_1'));
  const names = new Set();
  const ids = [];
  for (let i = 0; i < 3000; i++) ids.push(`call_${i}`);
  // Ids that only differ in characters a node name cannot carry, and long ids.
  ids.push('a.b', 'a_b', 'a/b', 'a b', 'x'.repeat(200), `${'x'.repeat(100)}y`, `${'x'.repeat(100)}z`);
  for (const id of ids) names.add(toolCallMirrorSlug(RUN, id));
  assert.equal(names.size, ids.length);
  // Runs that share their first 8 characters do not collide either.
  assert.notEqual(toolCallMirrorSlug('abcd1234-1', 'c'), toolCallMirrorSlug('abcd1234-2', 'c'));
  for (const n of names) assert.match(n, /^[A-Za-z0-9][A-Za-z0-9_-]{0,191}$/);
});

test('sendAgentOutboxMessage uses an explicit slug verbatim, validates it, and is idempotent on it', async () => {
  const { h } = world();
  await sendAgentOutboxMessage('ai', outbox, 't', 'ai_tool_call', {}, { slug: 'msg-custom-1' });
  await sendAgentOutboxMessage('ai', outbox, 't', 'ai_tool_call', {}, { slug: 'msg-custom-1' });
  assert.equal(h.calls.outbox.length, 1);
  assert.equal(h.calls.outbox[0].name, 'msg-custom-1');
  await assert.rejects(sendAgentOutboxMessage('ai', outbox, 't', 'ai_tool_call', {}, { slug: '../evil' }), /unsafe slug/);
  // Other types keep the hashed name.
  await sendAgentOutboxMessage('ai', outbox, 't', 'chat', {}, { dedupe_key: 'k' });
  assert.match(h.calls.outbox[1].name, /^msg-chat-[0-9a-f]+$/);
});

test('a hard stop writes the last turn\'s pending results once, before the final message', async () => {
  const { h, turn } = world({ calls: [{ call_id: 'c9', name: 'page.read', args: { a: 1 } }] });
  const input = {
    reason: 'final',
    finish: { outcome: 'blocked', reason: 'write_scope_violation', status_statement: 'I stopped.' },
    last_text: '',
    pending_tool_results: [{ call_id: 'c9', synthetic: false, content: { ok: true, content: [1] } }],
    __raisin_context: { workspace: 'ai', chat_path: CHAT, run_id: RUN, operation_id: `${RUN}/op/2` },
  };
  const env = await project(input);
  assert.equal(env.status, 'succeeded');
  assert.ok(await raisin.nodes.get('ai', `${turn.path}/tool-call-c9`));
  const order = h.calls.outbox.map((b) => b.properties.message_type);
  assert.deepEqual(order, ['ai_tool_call', 'chat']);
  // Idempotent: a re-delivered projection adds nothing.
  await project(input);
  assert.equal(h.calls.outbox.length, 2);
});

test('pendingToolResults: only what no model turn has been sent, never the tools-free refusals', () => {
  const st = (over) => ({ last_turn: { op_id: 'o', calls: ['a', 'b'] }, results: [{ call_id: 'a', synthetic: false, content: { ok: true } }], progress: { model_turns: 3 }, ...over });
  const p = pendingToolResults(st({}));
  assert.deepEqual(p.map((r) => r.call_id), ['a', 'b']);
  assert.equal(p[1].content.status, 'not_run');
  assert.deepEqual(pendingToolResults(st({ results_sent_turn: 3 })), []);
  assert.deepEqual(pendingToolResults(st({ last_turn: null })), []);
  const refused = st({ results: [{ call_id: 'a', synthetic: true, content: { error: 'No tools are offered in this turn.' } }] });
  assert.deepEqual(pendingToolResults(refused).map((r) => r.call_id), ['b']);
});

test('the Stop button lists the last turn\'s unanswered calls as stopped steps, once', async () => {
  const { h } = world({ calls: [{ call_id: 'c1', name: 'page.read', args: {} }, { call_id: 'c2', name: 'create_plan', args: {} }] });
  h.put('raisin:access_control', '/users/internal/u1/inbox/chats/chat-1', { node_type: 'raisin:Conversation', properties: { conversation_id: 'chat-1', stream_channel: 'chat:chat-1' } });
  h.put('raisin:access_control', '/users/internal/u1', { node_type: 'raisin:User', properties: { user_id: 'u1' } });
  h.nodes.get(`ai\u0000${CHAT}`).properties.active_agent_run_id = RUN;
  const origQuery = raisin.sql.query;
  raisin.sql.query = async (sql, params) => (sql.includes("node_type = 'raisin:Conversation'")
    ? [{ path: CHAT, properties: { active_agent_run_id: RUN } }] : origQuery(sql, params));
  const input = { conversation_path: '/users/internal/u1/inbox/chats/chat-1', stream_channel: 'chat:chat-1' };
  const r = await stop(input);
  assert.equal(r.accepted, true);
  const sent = steps(h);
  assert.equal(sent.length, 1, 'plan tools are not steps');
  assert.equal(sent[0].properties.data.status, 'stopped');
  await stop(input);
  assert.equal(steps(h).length, 1);
});

test('the Stop button records a call the run already had an answer for as done, and only the rest as stopped', async () => {
  const { h } = world({ calls: [{ call_id: 'c1', name: 'page.read', args: {} }, { call_id: 'c3', name: 'page.save', args: {} }] });
  h.put('raisin:access_control', '/users/internal/u1/inbox/chats/chat-1', { node_type: 'raisin:Conversation', properties: { conversation_id: 'chat-1', stream_channel: 'chat:chat-1' } });
  h.put('raisin:access_control', '/users/internal/u1', { node_type: 'raisin:User', properties: { user_id: 'u1' } });
  h.nodes.get(`ai\u0000${CHAT}`).properties.active_agent_run_id = RUN;
  h.runs[RUN].run.state = { last_turn: { calls: ['c1', 'c3'] }, results: [{ call_id: 'c1', synthetic: false, content: { ok: true, items: [1, 2] } }] };
  const origQuery = raisin.sql.query;
  raisin.sql.query = async (sql, params) => (sql.includes("node_type = 'raisin:Conversation'")
    ? [{ path: CHAT, properties: { active_agent_run_id: RUN } }] : origQuery(sql, params));
  const input = { conversation_path: '/users/internal/u1/inbox/chats/chat-1', stream_channel: 'chat:chat-1' };
  const r = await stop(input);
  assert.equal(r.accepted, true);
  const sent = steps(h);
  const byId = Object.fromEntries(sent.map((m) => [m.properties.data.tool_call_id, m.properties.data.status]));
  assert.deepEqual(byId, { c1: 'completed', c3: 'stopped' });
  assert.ok(h.nodes.get(`ai\u0000${CHAT}/run-abcd1234-op-1/tool-call-c1`), 'the answered call is in the agent transcript too');
  await stop(input);
  assert.equal(steps(h).length, 2);
});

test('created_at: the result\'s own finish time wins, else the write time keeps batch order', async () => {
  const { writeToolResults } = await import('../content/functions/lib/raisin/ai/agent-shared/run-transcript.js');
  const { h, turn } = world({ calls: [{ call_id: 'c1', name: 'a.x', args: {} }, { call_id: 'c2', name: 'a.y', args: {} }, { call_id: 'c3', name: 'a.z', args: {} }] });
  await writeToolResults({ workspace: 'ai', chatPath: CHAT, streamChannel: null, runId: RUN }, turn, [
    { call_id: 'c1', content: { ok: true }, finished_at: '2026-01-01T00:00:05Z' },
    { call_id: 'c2', content: { ok: true } },
    { call_id: 'c3', content: { ok: true } },
  ], { outbox });
  const at = steps(h).map((b) => b.properties.created_at);
  assert.equal(at[0], '2026-01-01T00:00:05.000Z');
  assert.ok(at[1] < at[2], 'fallback keeps order');
});

// ── backfill ────────────────────────────────────────────────────────────────

const ADMIN = { is_system_admin: true };
const asCaller = (caller) => { globalThis.raisin.context = { caller }; };
const OLD = [
  { id: 'k1', name: 'page.list', at: '2026-05-01T10:00:00.000Z', args: { q: 1 } },
  { id: 'k2', name: 'create_plan', at: '2026-05-01T10:00:01.000Z' },
  { id: 'k3', name: 'search-documents', at: '2026-05-01T10:00:02.000Z', status: 'failed', result: { status: 'failed', error: 'no index' } },
  { id: 'k4', name: 'page.read', at: '2026-05-01T10:00:03.000Z' },
];

test('backfill refuses every caller that is not a system admin', async () => {
  world({ stored: OLD });
  for (const caller of [null, { is_system: false, is_system_admin: false, user_id: 'u1' }, { anonymous: true }]) {
    asCaller(caller);
    const r = await backfill({});
    assert.equal(r.success, false);
    assert.equal(r.error, 'forbidden');
  }
  assert.equal((await raisin.nodes.getChildren('ai', `${AGENT}/outbox`)).length, 0);
});

test('backfill: dry run counts, a real run mirrors with the call\'s own time, a second run adds nothing', async () => {
  const { h } = world({ stored: OLD });
  asCaller(ADMIN);
  const dry = await backfill({ chat_path: CHAT, dry_run: true });
  assert.deepEqual([dry.scanned, dry.would_mirror, dry.skipped_internal, dry.mirrored], [4, 3, 1, 0]);
  assert.equal(steps(h).length, 0);
  const real = await backfill({ chat_path: CHAT });
  assert.equal(real.mirrored, 3);
  const sent = steps(h);
  assert.deepEqual(sent.map((b) => b.properties.created_at), ['2026-05-01T10:00:00.000Z', '2026-05-01T10:00:02.000Z', '2026-05-01T10:00:03.000Z']);
  assert.equal(sent[1].properties.data.status, 'failed');
  assert.equal(sent[1].properties.data.result_summary, 'no index');
  assert.equal(sent[0].name, toolCallMirrorSlug(RUN, 'k1'));
  const again = await backfill({ chat_path: CHAT });
  assert.deepEqual([again.mirrored, again.already_mirrored], [0, 3]);
  assert.equal(steps(h).length, 3);
});

test('backfill pages with a cursor and never reads past the limit', async () => {
  const { h } = world({ stored: OLD });
  asCaller(ADMIN);
  const p1 = await backfill({ agent_path: AGENT, limit: 2 });
  assert.equal(p1.scanned, 2);
  assert.equal(p1.done, false);
  assert.ok(p1.next_cursor);
  const p2 = await backfill({ agent_path: AGENT, limit: 2, cursor: p1.next_cursor });
  assert.equal(p2.scanned, 2);
  const p3 = await backfill({ agent_path: AGENT, limit: 2, cursor: p2.next_cursor });
  assert.equal(p3.scanned, 0);
  assert.equal(p3.done, true);
  assert.equal(steps(h).length, 3);
  const bad = await backfill({ chat_path: '/etc/passwd' });
  assert.equal(bad.error, 'invalid_input');
  assert.equal(chatPathOfCall(`${CHAT}/run-x-op-1/tool-call-a`), CHAT);
  assert.equal(chatPathOfCall('/agents/a/x'), null);
});

test('backfill and live mirroring agree: a live-mirrored step is not mirrored again', async () => {
  const { h, turn } = world({ calls: [{ call_id: 'k1', name: 'page.list', args: {} }], stored: [OLD[0]] });
  const { writeToolResults } = await import('../content/functions/lib/raisin/ai/agent-shared/run-transcript.js');
  await writeToolResults({ workspace: 'ai', chatPath: CHAT, streamChannel: null, runId: RUN }, turn, [{ call_id: 'k1', content: { ok: true } }], { outbox });
  asCaller(ADMIN);
  const r = await backfill({ chat_path: CHAT });
  // Both agent-side nodes of the call (the stored one and the live one) map to the one name.
  assert.deepEqual([r.mirrored, r.already_mirrored], [0, 2]);
  assert.equal(steps(h).length, 1);
});
