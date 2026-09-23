import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

const AGENT_CHAT = '/agents/builder/inbox/chats/one';

function fixture({ runStatus = 'running', channel = 'chat:one', runId = 'run-live0001' } = {}) {
  const updates = [];
  const controls = [];
  globalThis.raisin = {
    nodes: {
      async get(workspace, path) {
        if (workspace === 'raisin:access_control' && path === '/users/u/inbox/chats/one') {
          return { path, node_type: 'raisin:Conversation', properties: { conversation_id: 'one', stream_channel: channel } };
        }
        if (workspace === 'ai' && path === AGENT_CHAT) {
          return { path, node_type: 'raisin:Conversation', properties: { active_agent_run_id: runId } };
        }
        return null;
      },
      async create(workspace, parent, body) {
        updates.push({ workspace, path: `${parent}/${body.name}`, created: true });
        return { path: `${parent}/${body.name}`, ...body };
      },
      async updateProperty(workspace, path, key, value) {
        updates.push({ workspace, path, key, value });
      },
    },
    sql: {
      async query(sql) {
        if (sql.includes("node_type = 'raisin:Conversation'")) {
          return [{ path: AGENT_CHAT, properties: runId ? { active_agent_run_id: runId } : {} }];
        }
        return [];
      },
    },
    events: { async emit() {} },
    crypto: { async uuid() { return 'request-1'; } },
    agentRuns: {
      async create() { return null; },
      async get({ run_id }) {
        if (run_id !== runId) throw new Error('not_found: run not found');
        return { status: runStatus, run: { run_id, subject: { workspace: 'ai', path: AGENT_CHAT } } };
      },
      async control(req) {
        controls.push(req);
        return { ack: 'applied', seq: 7 };
      },
    },
  };
  return { updates, controls };
}

test('a live run is stopped through the runtime, and acknowledged', async () => {
  const { controls } = fixture();
  const result = await handler({ conversation_path: '/users/u/inbox/chats/one', stream_channel: 'chat:one' });
  assert.equal(result.accepted, true);
  assert.equal(result.run_id, 'run-live0001');
  assert.equal(controls.length, 1);
  assert.equal(controls[0].command.command, 'stop');
  assert.equal(controls[0].control_id, 'stop:request-1');
});

test('a finished run is not running: nothing is controlled or written', async () => {
  const { updates, controls } = fixture({ runStatus: 'completed' });
  const result = await handler({ conversation_path: '/users/u/inbox/chats/one', stream_channel: 'chat:one' });
  assert.equal(result.accepted, false);
  assert.match(result.message, /not currently running/i);
  assert.equal(controls.length, 0);
  assert.equal(updates.length, 0);
});

test('a conversation without a run is not running', async () => {
  const { controls } = fixture({ runId: null });
  const result = await handler({ conversation_path: '/users/u/inbox/chats/one', stream_channel: 'chat:one' });
  assert.equal(result.accepted, false);
  assert.equal(controls.length, 0);
});

test('refuses a mismatched stream capability', async () => {
  const { updates } = fixture();
  await assert.rejects(
    () => handler({ conversation_path: '/users/u/inbox/chats/one', stream_channel: 'chat:wrong' }),
    /does not match/i,
  );
  assert.equal(updates.length, 0);
});
