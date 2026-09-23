/**
 * A model's tool call runs ONLY through the tools it was offered.
 *
 * Under a run the reducer refuses an unoffered call (agent-run-reducer.test),
 * core refuses a call outside a child's `allowed_tools`, and the model-turn
 * function does not even offer one (`allowedOffers`). Runtime-only keys a
 * model invents never reach a function (`stripRuntimeArgs`, the JS half of
 * `strip_runtime_keys` in crates/raisin-flow-runtime).
 *
 * Run: node --test builtin-packages/ai-tools/tests/offered-tools.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { stripRuntimeArgs } from '../content/functions/lib/raisin/ai/agent-shared/tools.js';
import { toolAllowed, allowedOffers } from '../content/functions/lib/raisin/ai/agent-shared/run-tools.js';

const def = (name) => ({ type: 'function', function: { name, description: '', parameters: { type: 'object', properties: {} } } });
const ref = (path) => ({ 'raisin:ref': `id-${path}`, 'raisin:workspace': 'functions', 'raisin:path': path, execution_mode: 'async', category: null });

// ── The pure half ───────────────────────────────────────────────────────────

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

test('allowed_tools: exact paths and `*` prefixes, as core matches them; empty allows all', () => {
  assert.equal(toolAllowed(['/lib/a/read'], '/lib/a/read'), true);
  assert.equal(toolAllowed(['/lib/a/read'], '/lib/a/write'), false);
  assert.equal(toolAllowed(['/lib/a/*'], '/lib/a/write'), true);
  assert.equal(toolAllowed(['/lib/a/*'], '/lib/b/write'), false);
  assert.equal(toolAllowed([], '/lib/b/write'), true);
  assert.equal(toolAllowed(undefined, '/lib/b/write'), true);
});

test('a child run is offered only its granted function tools; planning (domain) tools stay', () => {
  const offers = [
    { name: 'search', kind: 'function', function_path: '/lib/demo/search' },
    { name: 'write', kind: 'function', function_path: '/lib/demo/write' },
    { name: 'create_plan', kind: 'domain', function_path: null },
  ];
  const kept = allowedOffers(offers, ['/lib/demo/search', '/lib/raisin/ai/agent-run-project']);
  assert.deepEqual(kept.map((t) => t.name), ['search', 'create_plan']);
  assert.equal(allowedOffers(offers, null).length, 3);
});
