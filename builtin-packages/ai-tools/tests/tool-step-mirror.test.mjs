// A FINISHED TOOL CALL LEAVES ONE SMALL MESSAGE IN THE PERSON'S CONVERSATION.
//
// The engine keeps each call under the agent's own message, where the chatting
// person's row-level security cannot read it, so the steps of a run vanished on
// reload. writeToolResults now also drops an `ai_tool_call` message into the
// agent's outbox (the messaging trigger mirrors it, as it does the plan card).
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from './support/fake-raisin.mjs';

fakeRaisin();
const { writeToolResults } = await import('../content/functions/lib/raisin/ai/agent-shared/run-transcript.js');
const { boundArguments, resultSummary, mirrorStatus, isMirroredCall, SUMMARY_MAX_CHARS, ARGS_MAX_CHARS } = await import(
  '../content/functions/lib/raisin/ai/agent-shared/run-tool-mirror.js'
);

const CHAT = '/agents/site/inbox/chats/chat-1';
const outbox = { agentHomePath: '/agents/site', agentUserId: 'agent:site', agentDisplayName: 'Site', senderId: 'u1', senderPath: '/users/internal/u1', conversationId: 'chat-1' };

function setup(calls) {
  const h = fakeRaisin();
  h.put('ai', '/agents/site', { node_type: 'raisin:Folder', properties: {} });
  h.put('ai', CHAT, { node_type: 'raisin:Conversation', properties: {} });
  const prev = h.put('ai', `${CHAT}/run-abcd1234-op-1`, { node_type: 'raisin:Message', properties: { run_tool_calls: calls } });
  return { h, prev };
}
const ctx = { workspace: 'ai', chatPath: CHAT, streamChannel: null, runId: 'abcd1234-run' };

test('each finished call is mirrored once, with the documented shape', async () => {
  const { h, prev } = setup([
    { call_id: 'c1', name: 'page.list_agenda', args: { day: 'friday' } },
    { call_id: 'c2', name: 'search-documents', args: { q: 'x' } },
  ]);
  await writeToolResults(ctx, prev, [
    { call_id: 'c1', content: { ok: true, content: [1, 2, 3] } },
    { call_id: 'c2', content: { status: 'failed', error: 'no index' } },
  ], { outbox });
  const sent = h.calls.outbox.filter((b) => b.properties.message_type === 'ai_tool_call');
  assert.equal(sent.length, 2);
  const [a, b] = sent.map((m) => m.properties);
  assert.equal(a.role, 'assistant');
  assert.equal(a.recipient_id, 'u1');
  assert.deepEqual(a.data, {
    tool_call_id: 'c1', tool_name: 'page.list_agenda', function_name: 'page.list_agenda',
    arguments: { day: 'friday' }, status: 'completed', run_id: 'abcd1234-run',
  });
  assert.equal(b.data.status, 'failed');
  assert.equal('result_summary' in b.data, false, 'no result for an agent that does not stream results');
  assert.ok(a.created_at < b.created_at, 'calls of one batch keep their order');
});

test('a result summary is stored only for an agent that streams results (live or recorded on the turn)', async () => {
  const live = setup([{ call_id: 'c1', name: 'search-documents', args: {} }]);
  await writeToolResults(ctx, live.prev, [{ call_id: 'c1', content: { status: 'failed', error: 'no index' } }], { outbox, streamResults: true });
  assert.equal(live.h.calls.outbox.find((b) => b.properties.message_type === 'ai_tool_call').properties.data.result_summary, 'no index');
  const recorded = setup([{ call_id: 'c2', name: 'page.read', args: {} }]);
  recorded.prev.properties.run_stream_results = true;
  await writeToolResults(ctx, recorded.prev, [{ call_id: 'c2', content: { ok: true, content: [1, 2, 3] } }], { outbox });
  assert.equal(recorded.h.calls.outbox.find((b) => b.properties.message_type === 'ai_tool_call').properties.data.result_summary, '3 items');
});

test('re-projection does not duplicate (replay writes nothing new)', async () => {
  const { h, prev } = setup([{ call_id: 'c1', name: 'page.read', args: {} }]);
  const results = [{ call_id: 'c1', content: { ok: true } }];
  await writeToolResults(ctx, prev, results, { outbox });
  await writeToolResults(ctx, prev, results, { outbox });
  assert.equal(h.calls.outbox.filter((b) => b.properties.message_type === 'ai_tool_call').length, 1);
});

test('a call whose agent-side node exists but whose mirror is missing is mirrored on replay', async () => {
  const { h, prev } = setup([{ call_id: 'c1', name: 'page.read', args: {} }]);
  h.put('ai', `${prev.path}/tool-call-c1`, { node_type: 'raisin:AIToolCall', properties: {} });
  await writeToolResults(ctx, prev, [{ call_id: 'c1', content: { ok: true } }], { outbox });
  assert.equal(h.calls.outbox.length, 1);
});

test('plan tools are not mirrored (the plan card shows them); other synthetic calls are', async () => {
  const { h, prev } = setup([
    { call_id: 'p1', name: 'create_plan', args: { title: 't' } },
    { call_id: 'p2', name: 'update-task', args: {} },
    { call_id: 'p3', name: 'ask-user', args: {} },
    { call_id: 'p4', name: '/lib/raisin/ai/agent-run-project', args: {} },
    { call_id: 'c1', name: 'page.create_session', args: {} },
  ]);
  await writeToolResults(ctx, prev, [
    { call_id: 'p1', synthetic: true, content: { success: true } },
    { call_id: 'p2', synthetic: true, content: { success: true } },
    { call_id: 'p3', synthetic: true, content: { success: true } },
    { call_id: 'p4', synthetic: true, content: {} },
    { call_id: 'c1', synthetic: false, content: { ok: false, error: 'declined', code: 'declined', message: 'The person declined this action.' } },
  ], { outbox });
  const names = h.calls.outbox.map((b) => b.properties.data.tool_name);
  assert.deepEqual(names, ['page.create_session']);
  assert.equal(h.calls.outbox[0].properties.data.status, 'declined');
  assert.equal(isMirroredCall('create-plan'), false);
});

test('no outbox context (visitor, non-agent chat): nothing mirrored, results still written', async () => {
  const { h, prev } = setup([{ call_id: 'c1', name: 'page.read', args: {} }]);
  const n = await writeToolResults(ctx, prev, [{ call_id: 'c1', content: { ok: true } }], { outbox: null });
  assert.equal(n, 1);
  assert.equal(h.calls.outbox.length, 0);
});

test('size is bounded and secrets are masked', () => {
  const big = boundArguments({ text: 'y'.repeat(5000), api_key: 'sk-123', nested: { password: 'p', list: Array.from({ length: 100 }, (_, i) => i) } });
  assert.ok(JSON.stringify(big).length <= ARGS_MAX_CHARS);
  assert.equal(big.api_key, '[hidden]');
  assert.equal(big.nested.password, '[hidden]');
  assert.ok(big.text.length < 400);
  const wide = boundArguments(Object.fromEntries(Array.from({ length: 400 }, (_, i) => [`k${i}`, 'v'.repeat(200)])));
  assert.ok(JSON.stringify(wide).length <= ARGS_MAX_CHARS);
  assert.ok(resultSummary({ blob: 'z'.repeat(9000) }).length <= SUMMARY_MAX_CHARS);
  assert.ok(resultSummary('q'.repeat(9000)).length <= SUMMARY_MAX_CHARS);
  assert.equal(mirrorStatus({ error: 'expired' }), 'expired');
  assert.equal(mirrorStatus({ status: 'not_run' }), 'failed');
  assert.equal(mirrorStatus({ ok: true }), 'completed');
});
