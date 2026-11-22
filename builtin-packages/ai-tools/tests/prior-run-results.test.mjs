/** prior_run_tool_result_chars: earlier runs' tool results are shortened, the current run's are not. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from './support/fake-raisin.mjs';
import { buildHistoryFromChat } from '../content/functions/lib/raisin/ai/agent-shared/history.js';
import { shortenPriorRunResults, stripRunTags } from '../content/functions/lib/raisin/ai/agent-run-model-turn/context-prior.js';
import { applyBudget } from '../content/functions/lib/raisin/ai/agent-run-model-turn/context.js';

const CHAT = '/agents/helper/inbox/chats/c1';
const RUNS = ['aaaaaaaa1111', 'bbbbbbbb2222', 'cccccccc3333'];
const BIG = 'r'.repeat(3000);

/** Three runs in one chat; each: user message, assistant text + one tool call with a big result. */
function seed() {
  const f = fakeRaisin();
  f.put('ai', CHAT, { node_type: 'raisin:Conversation', properties: {} });
  RUNS.forEach((run, i) => {
    f.put('ai', `${CHAT}/u${i}`, { node_type: 'raisin:Message', properties: { role: 'user', body: `question ${i}` } });
    const msg = `${CHAT}/run-${run.slice(0, 8)}-op-1`;
    f.put('ai', msg, { node_type: 'raisin:Message', properties: { role: 'assistant', body: `answer ${i}`, run_id: run } });
    f.put('ai', `${msg}/call-1`, { node_type: 'raisin:AIToolCall', properties: { tool_call_id: `c${i}`, function_name: 'look', arguments: { q: i } } });
    f.put('ai', `${msg}/call-1/result`, { node_type: 'raisin:AIToolResult', properties: { result: `${i}:${BIG}` } });
  });
  return f;
}

const build = (opts) => buildHistoryFromChat('ai', CHAT, 'sys', null, null, opts);
const strip = (h) => h.map((e) => { const { __run_id, ...r } = e; return r; });

test('property absent: history is identical (no markers, no shortening)', async () => {
  seed();
  const plain = await build({});
  assert.ok(!plain.some((e) => '__run_id' in e));
  const r = shortenPriorRunResults(plain, RUNS[2], 0);
  assert.equal(r.history, plain);
  assert.equal(r.shortened, 0);
});

test('tagged history differs from plain only by the marker', async () => {
  seed();
  assert.deepEqual(strip(await build({ tagRuns: true })), await build({}));
});

test('earlier-run results are shortened; current run, calls and assistant text stay', async () => {
  seed();
  const tagged = await build({ tagRuns: true });
  const { history, shortened } = shortenPriorRunResults(tagged, RUNS[2], 100);
  assert.equal(shortened, 2);
  const tools = history.filter((e) => e.role === 'tool');
  assert.equal(tools.length, 3);
  assert.match(tools[0].content, /^"0:r+/);
  assert.match(tools[0].content, /\[earlier run: 100 of \d+ characters kept; call the tool again/);
  assert.ok(tools[0].content.length < 300);
  assert.ok(tools[1].content.includes('[earlier run:'));
  assert.equal(tools[2].content, tagged.filter((e) => e.role === 'tool')[2].content, 'current run untouched');
  // calls and assistant text intact
  const before = tagged.filter((e) => e.role === 'assistant');
  const after = history.filter((e) => e.role === 'assistant');
  assert.deepEqual(after, before);
  assert.deepEqual(after.map((a) => a.content), ['answer 0', 'answer 1', 'answer 2']);
});

test('results shorter than N are unchanged', async () => {
  seed();
  const tagged = await build({ tagRuns: true });
  const r = shortenPriorRunResults(tagged, RUNS[2], 100000);
  assert.equal(r.shortened, 0);
  assert.deepEqual(r.history, tagged);
});

test('pairing preserved: every tool result follows its assistant call, same ids and order', async () => {
  seed();
  const tagged = await build({ tagRuns: true });
  const { history } = shortenPriorRunResults(tagged, RUNS[1], 50);
  const ids = (h) => h.flatMap((e) => (e.tool_calls ? e.tool_calls.map((c) => `call:${c.id}`) : e.role === 'tool' ? [`res:${e.tool_call_id}`] : []));
  assert.deepEqual(ids(history), ids(tagged));
  assert.deepEqual(ids(history), ['call:c0', 'res:c0', 'call:c1', 'res:c1', 'call:c2', 'res:c2']);
  // run 2 (middle) is current: run 1 and 3 are "other" runs
  const tools = history.filter((e) => e.role === 'tool');
  assert.ok(tools[0].content.includes('[earlier run:'));
  assert.ok(!tools[1].content.includes('[earlier run:'));
});

test('attribution falls back to the run-<short>-op node name; untagged results count as earlier', async () => {
  const f = fakeRaisin();
  f.put('ai', CHAT, { node_type: 'raisin:Conversation', properties: {} });
  const msg = `${CHAT}/run-aaaaaaaa-op-1`;
  f.put('ai', msg, { node_type: 'raisin:Message', properties: { role: 'assistant', body: 'x' } }); // no run_id property
  f.put('ai', `${msg}/call-1`, { node_type: 'raisin:AIToolCall', properties: { tool_call_id: 'c', function_name: 'look', arguments: {} } });
  f.put('ai', `${msg}/call-1/result`, { node_type: 'raisin:AIToolResult', properties: { result: BIG } });
  const tagged = await build({ tagRuns: true });
  assert.equal(tagged.find((e) => e.role === 'tool').__run_id, 'aaaaaaaa');
  assert.equal(shortenPriorRunResults(tagged, RUNS[0], 100).shortened, 0, 'same run by short id');
  assert.equal(shortenPriorRunResults(tagged, RUNS[1], 100).shortened, 1);
  assert.equal(shortenPriorRunResults([{ role: 'tool', content: BIG, tool_call_id: 'z' }], RUNS[0], 100).shortened, 1);
});

test('three runs: shortening happens before the budget and markers are stripped', async () => {
  seed();
  const tagged = await build({ tagRuns: true });
  const { history } = shortenPriorRunResults(tagged, RUNS[2], 100);
  const out = applyBudget(history, { budgetTokens: 100000 });
  const clean = stripRunTags(out.messages);
  assert.ok(!clean.some((e) => '__run_id' in e));
  assert.ok(out.tokens < applyBudget(tagged, { budgetTokens: 100000 }).tokens / 2 + 600);
});
