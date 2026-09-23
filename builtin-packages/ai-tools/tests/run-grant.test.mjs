/**
 * The run's grant: an agent node's `node_dev.roots` reach the run RECORD
 * (`executor_config.node_dev.roots`, which core narrows every node-development
 * call of the run's tools by) and the reducer (`input.config.node_dev`), and
 * nothing else can widen them.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from './support/fake-raisin.mjs';

fakeRaisin();
const { buildCreateRequest, nodeDevGrant } = await import('../content/functions/lib/raisin/ai/agent-shared/run-entry.js');

const base = {
  workspace: 'ai', chatPath: '/agents/studio-builder/inbox/chats/c1', chat: { id: 'chat-1' },
  message: { id: 'm1', path: '/agents/studio-builder/inbox/chats/c1/m1', properties: { content: 'build it' } },
  agentRef: 'functions:/agents/studio-builder', tools: [], plan: null, senderId: 'user-1', actingUser: 'user-1',
};

test('an agent node\'s node_dev roots become the run\'s grant and the reducer\'s bound', () => {
  const agentProps = {
    execution_mode: 'approve_then_auto',
    node_dev: { roots: [
      { workspace: 'automations', path: '/', ops: ['read', 'create', 'update'] },
      { workspace: 'functions', ops: ['read'] },
      { path: '/no-workspace' },
    ] },
  };
  const req = buildCreateRequest({ ...base, agentProps });
  const roots = [
    { workspace: 'automations', path: '/', ops: ['read', 'create', 'update'] },
    { workspace: 'functions', path: '/', ops: ['read'] },
  ];
  assert.deepEqual(req.executor_config.node_dev, { roots }, 'the run record carries the grant core enforces');
  assert.deepEqual(req.input.config.node_dev, { roots }, 'the reducer sees the same roots');
});

test('an agent without node_dev roots runs with no grant — and run_config cannot smuggle one into the record', () => {
  const req = buildCreateRequest({ ...base, agentProps: { run_config: { node_dev: { roots: [{ workspace: 'x' }] } } } });
  assert.equal(req.executor_config.node_dev, undefined);
  assert.equal(nodeDevGrant({ node_dev: { roots: [] } }), null);
});
