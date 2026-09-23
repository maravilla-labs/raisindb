// A MODEL'S BAD REFERENCE MUST NOT STOP THE CONVERSATION.
//
// Measured in a Builder run: propose-automation was called with
// {raisin:ref: /agents/homepage-title-reviewer, raisin:workspace: agents} — the
// wrong workspace. The server refused the tool-call node ("Referenced node not
// found") and the conversation stopped with nothing for the model to correct.
// A run's transcript writes (the assistant turn with its calls, each call and
// its result) are stored once more with plain paths instead.
import test from 'node:test';
import assert from 'node:assert/strict';

import { createOrGet } from '../content/functions/lib/raisin/ai/agent-shared/run-transcript.js';
import { plainReferences } from '../content/functions/lib/raisin/ai/agent-shared/utils.js';

function host(refuse) {
  const writes = [];
  globalThis.raisin = {
    nodes: {
      async create(ws, parent, def) {
        writes.push(def);
        if (refuse(def)) throw new Error('Validation failed: Referenced node not found: agents:/agents/x');
        return { path: `${parent}/${def.name}`, properties: def.properties };
      },
      async get() { return null; },
    },
  };
  return writes;
}
const hasEnvelope = (v) => JSON.stringify(v).includes('raisin:ref');
const call = (args) => ({ name: 'tool-call-1', node_type: 'raisin:AIToolCall', properties: { tool_call_id: 't1', function_name: 'propose-automation', arguments: args, status: 'completed' } });

test('a bad reference in the arguments is stored as a plain path, and the transcript goes on', async () => {
  const writes = host((def) => hasEnvelope(def.properties));
  const args = { candidate: { steps: [{ agent: { 'raisin:ref': '/agents/x', 'raisin:workspace': 'agents' } }] } };
  const node = await createOrGet('ai', '/chat/m1', call(args));
  assert.equal(writes.length, 2, 'written once more');
  assert.deepEqual(node.properties.arguments, { candidate: { steps: [{ agent: '/agents/x' }] } });
  assert.equal(node.properties.status, 'completed', 'a final status: core never executes it');
  assert.match(node.properties.references_rewritten, /Referenced node not found/);
});

test('good arguments are written once, untouched', async () => {
  const writes = host(() => false);
  const args = { agent: { 'raisin:ref': '/agents/y', 'raisin:workspace': 'functions' } };
  const node = await createOrGet('ai', '/chat/m1', call(args));
  assert.equal(writes.length, 1);
  assert.deepEqual(node.properties.arguments, args);
});

test('any other failure still throws', async () => {
  globalThis.raisin = { nodes: { async create() { throw new Error('disk full'); } } };
  await assert.rejects(createOrGet('ai', '/c', call({})), /disk full/);
});

test('plainReferences keeps the path when the envelope has one', () => {
  assert.equal(plainReferences({ 'raisin:ref': 'id-1', 'raisin:path': '/agents/z', 'raisin:workspace': 'functions' }), '/agents/z');
  assert.deepEqual(plainReferences([1, 'a', { b: null }]), [1, 'a', { b: null }]);
});
