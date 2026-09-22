// A MODEL'S BAD REFERENCE MUST NOT STOP THE CONVERSATION.
//
// Measured in a Builder run: propose-automation was called with
// {raisin:ref: /agents/homepage-title-reviewer, raisin:workspace: agents} — the
// wrong workspace. The server refused the tool-call node ("Referenced node not
// found"), the continuation ended in its catch, and the conversation stopped
// with nothing for the model to correct.
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const AI = join(dirname(fileURLToPath(import.meta.url)), '../content/functions/lib/raisin/ai');
const { createToolCallNode, plainReferences } = await import(`${AI}/agent-shared/utils.js`);

function host(refuse) {
  const writes = [];
  globalThis.raisin = { nodes: { async create(ws, parent, def) {
    writes.push(def);
    if (refuse(def)) throw new Error('Validation failed: Referenced node not found: agents:/agents/x');
    return { path: `${parent}/${def.name}` };
  } } };
  return writes;
}
const hasEnvelope = (v) => JSON.stringify(v).includes('raisin:ref');
const props = (args) => ({ tool_call_id: 't1', function_name: 'propose-automation', function_ref: {}, arguments: args, status: 'pending' });

test('a bad reference in the arguments is stored as a plain path, and the tool still runs', async () => {
  const writes = host((def) => hasEnvelope(def.properties.arguments));
  const args = { candidate: { steps: [{ agent: { 'raisin:ref': '/agents/x', 'raisin:workspace': 'agents' } }] } };
  const out = await createToolCallNode('ai', '/chat/m1', 'tool-call-1', props(args));
  assert.equal(writes.length, 2, 'written once more');
  assert.deepEqual(out.args, { candidate: { steps: [{ agent: '/agents/x' }] } });
  assert.equal(writes[1].properties.status, 'pending', 'still pending, so the tool executes');
  assert.match(writes[1].properties.arguments_rewritten, /Referenced node not found/);
});

test('good arguments are written once, untouched', async () => {
  const writes = host(() => false);
  const args = { agent: { 'raisin:ref': '/agents/y', 'raisin:workspace': 'functions' } };
  const out = await createToolCallNode('ai', '/chat/m1', 'tool-call-1', props(args));
  assert.equal(writes.length, 1);
  assert.deepEqual(out.args, args);
});

test('any other failure still throws', async () => {
  globalThis.raisin = { nodes: { async create() { throw new Error('disk full'); } } };
  await assert.rejects(createToolCallNode('ai', '/c', 'n', props({})), /disk full/);
});

test('plainReferences keeps the path when the envelope has one', () => {
  assert.equal(plainReferences({ 'raisin:ref': 'id-1', 'raisin:path': '/agents/z', 'raisin:workspace': 'functions' }), '/agents/z');
  assert.deepEqual(plainReferences([1, 'a', { b: null }]), [1, 'a', { b: null }]);
});
