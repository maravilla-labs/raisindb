/**
 * The tool-loop guard, which used to fire on agents that were working.
 *
 * `detectToolLoop` is module-private on purpose — nothing but the continue
 * handler should call it — so the source is loaded and re-exported through a
 * data: URL rather than widening the function's surface for a test.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-loop-guard.test.js
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const SOURCE = join(HERE, '../content/functions/lib/raisin/ai/agent-continue-handler/index.js');

const mod = await import(SOURCE);

/** One assistant round that called `name` with `args`. */
const round = (name, args) => ({
  role: 'assistant',
  tool_calls: [{ function: { name, arguments: JSON.stringify(args) } }],
});
const toolResult = () => ({ role: 'tool', content: 'ok' });

/** A history of alternating assistant rounds and their tool results. */
const history = (...rounds) => rounds.flatMap((r) => [r, toolResult()]);

test('three identical calls are a loop', () => {
  const h = history(
    round('discover-capabilities', { query: 'mail' }),
    round('discover-capabilities', { query: 'mail' }),
    round('discover-capabilities', { query: 'mail' }),
  );
  assert.equal(mod.detectToolLoop(h), 'discover-capabilities');
});

/**
 * THE REGRESSION. The signature was the tool NAME only, so a search tool keyed
 * on free text looked identical no matter what was searched for. An agent
 * working a five-task plan — one lookup per task — tripped the guard on task
 * three, had its tools stripped, and told the user it "kept calling
 * discover-capabilities without getting anywhere" after three successful,
 * different, useful searches.
 */
test('the same tool asked three different questions is not a loop', () => {
  const h = history(
    round('discover-capabilities', { query: 'draft reply agent' }),
    round('discover-capabilities', { query: 'inbox trigger' }),
    round('discover-capabilities', { query: 'escalation rule' }),
  );
  assert.equal(mod.detectToolLoop(h), null);
});

test('two identical calls are not yet a loop', () => {
  const h = history(
    round('find-nodes', { query: 'x' }),
    round('find-nodes', { query: 'x' }),
  );
  assert.equal(mod.detectToolLoop(h), null);
});

test('one differing call in the last three breaks the run', () => {
  const h = history(
    round('find-nodes', { query: 'x' }),
    round('find-nodes', { query: 'x' }),
    round('find-nodes', { query: 'y' }),
  );
  assert.equal(mod.detectToolLoop(h), null);
});

test('key order does not make one call look like two', () => {
  const a = { role: 'assistant', tool_calls: [{ function: { name: 't', arguments: '{"a":1,"b":2}' } }] };
  const b = { role: 'assistant', tool_calls: [{ function: { name: 't', arguments: '{"b":2,"a":1}' } }] };
  const h = history(a, b, a);
  assert.equal(mod.detectToolLoop(h), 't', 'the same call spelled two ways is still the same call');
});

test('arguments that are not JSON are compared as sent, not discarded', () => {
  const bad = (raw) => ({ role: 'assistant', tool_calls: [{ function: { name: 't', arguments: raw } }] });
  assert.equal(mod.detectToolLoop(history(bad('<<not json'), bad('<<not json'), bad('<<not json'))), 't');
  // Falling back to the name would call these three a loop. They are not.
  assert.equal(mod.detectToolLoop(history(bad('<<a'), bad('<<b'), bad('<<c'))), null);
});

test('a user message stops the scan, so a new turn starts clean', () => {
  const h = [
    ...history(round('t', { q: 1 })),
    { role: 'user', content: 'do it again' },
    ...history(round('t', { q: 1 }), round('t', { q: 1 })),
  ];
  assert.equal(mod.detectToolLoop(h), null);
});

test('a multi-call round compares as a set, order-independently', () => {
  const ab = {
    role: 'assistant',
    tool_calls: [
      { function: { name: 'a', arguments: '{}' } },
      { function: { name: 'b', arguments: '{}' } },
    ],
  };
  const ba = {
    role: 'assistant',
    tool_calls: [
      { function: { name: 'b', arguments: '{}' } },
      { function: { name: 'a', arguments: '{}' } },
    ],
  };
  assert.ok(mod.detectToolLoop(history(ab, ba, ab)), 'same pair, either order, is one round');
});

test('a call with no arguments still compares by name', () => {
  const none = { role: 'assistant', tool_calls: [{ function: { name: 'get-plan-status' } }] };
  assert.equal(mod.detectToolLoop(history(none, none, none)), 'get-plan-status');
});

/**
 * The caller WITHDRAWS the tools this names, rather than taking every tool
 * away, so the names must be complete and must not carry arguments.
 *
 * Stripping everything made the agent's own instructions unsatisfiable at the
 * moment they mattered: `update-task` went with the rest, so "close every task
 * you open" became impossible and every guard trip left its tasks in_progress
 * by construction.
 */
test('a looping round names every tool in it, and only the names', () => {
  const pair = {
    role: 'assistant',
    tool_calls: [
      { function: { name: 'find-nodes', arguments: '{"q":"a"}' } },
      { function: { name: 'discover-capabilities', arguments: '{"query":"b"}' } },
    ],
  };
  const names = mod.detectToolLoop(history(pair, pair, pair));
  assert.deepEqual(names.split(',').sort(), ['discover-capabilities', 'find-nodes']);
  assert.ok(!names.includes('{'), 'arguments must not leak into the withdrawn-tool names');
  assert.ok(!names.includes(':'), 'a signature must not leak into the withdrawn-tool names');
});

test('a single looping tool names exactly itself', () => {
  const h = history(
    round('discover-capabilities', { query: 'x' }),
    round('discover-capabilities', { query: 'x' }),
    round('discover-capabilities', { query: 'x' }),
  );
  assert.equal(mod.detectToolLoop(h), 'discover-capabilities');
});

// ── The tool-round budget ────────────────────────────────────────────────
/**
 * A build is not a chat. Twenty rounds is generous for a conversation and thin
 * for a job: discover, propose, create, materialize, validate, fix,
 * re-materialize, re-validate is eight before any retry. An agent that runs
 * out ends its turn mid-work with "I've reached the maximum number of tool
 * continuation steps" and leaves half a thing behind, so an agent that does
 * long jobs can say so.
 */
test('an agent with no opinion gets the default', () => {
  assert.equal(mod.continuationBudget(undefined), 20);
  assert.equal(mod.continuationBudget({}), 20);
  assert.equal(mod.continuationBudget({ max_tool_rounds: null }), 20);
});

test('an agent that asks for more rounds gets them', () => {
  assert.equal(mod.continuationBudget({ max_tool_rounds: 80 }), 80);
  assert.equal(mod.continuationBudget({ max_tool_rounds: '80' }), 80, 'a number from YAML may arrive as a string');
});

/** The budget is also the only thing between a confused agent and a bill. */
test('the override is capped', () => {
  assert.equal(mod.continuationBudget({ max_tool_rounds: 5000 }), 200);
});

test('nonsense falls back rather than granting an unbounded turn', () => {
  for (const bad of [0, -1, 'lots', NaN, {}, []]) {
    assert.equal(mod.continuationBudget({ max_tool_rounds: bad }), 20, `${JSON.stringify(bad)} should fall back`);
  }
});

test('a fractional budget floors rather than rounding up', () => {
  assert.equal(mod.continuationBudget({ max_tool_rounds: 20.9 }), 20);
});

// ── Budget measured from progress ────────────────────────────────────────
/**
 * The arithmetic the depth check does, spelled out.
 *
 * The cap defends against a model going round in circles. A turn ticking tasks
 * off a plan is demonstrably not doing that, so counting its rounds against a
 * flat per-turn budget punishes the work for taking the steps it takes — an
 * eight-task plan spent its allowance on task five and stopped, and the user
 * typed "go ahead", which starts a new base message and hands back a full
 * budget for nothing. A limit any reply resets is theatre.
 */
const since = (nextCount, progressDepth) =>
  (Number.isFinite(progressDepth) && progressDepth > 0 ? nextCount - progressDepth : nextCount);

test('with no progress the budget is the whole turn', () => {
  assert.equal(since(21, 0), 21, 'never progressed: every round counts');
  assert.equal(since(21, undefined), 21);
});

test('a completed task moves the start line', () => {
  // Task finished at round 18; round 21 is the third round since.
  assert.equal(since(21, 18), 3);
});

/**
 * The guard on the guard: an agent that loops WITHOUT finishing anything still
 * runs out in exactly the budget, because the mark only moves on a real
 * completion.
 */
test('looping without finishing anything still runs out', () => {
  const budget = 20;
  // Last real progress at round 5, then nothing but spinning.
  assert.equal(since(25, 5) > budget, false, '20 rounds after progress is still inside');
  assert.equal(since(26, 5) > budget, true, 'the 21st is not');
});

test('progress right before the wall buys exactly one full budget more', () => {
  const budget = 20;
  assert.equal(since(20, 20) > budget, false);
  assert.equal(since(40, 20) > budget, false, 'a full budget after the mark');
  assert.equal(since(41, 20) > budget, true);
});
