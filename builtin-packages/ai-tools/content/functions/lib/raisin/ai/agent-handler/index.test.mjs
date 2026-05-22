import assert from 'node:assert/strict';
import test from 'node:test';

import { handleUserMessage, findInFlightAssistantTurn } from './index.js';

test('finally safety-net does not shadow original tool-processing error', async () => {
  const messagePath = '/agents/sample-assistant/inbox/chats/chat-1/msg-1';
  const chatPath = '/agents/sample-assistant/inbox/chats/chat-1';
  const replyPath = `${chatPath}/reply-to-msg-1`;

  globalThis.raisin = {
    nodes: {
      async get(workspace, path) {
        if (path === messagePath) {
          return {
            path,
            node_type: 'raisin:Message',
            properties: {
              role: 'user',
              message_type: 'chat',
              status: 'delivered',
              content: 'hello',
            },
          };
        }
        if (path === chatPath) {
          return {
            path,
            node_type: 'raisin:Conversation',
            properties: {
              agent_ref: '/agents/sample-assistant',
              participants: ['user-1'],
              human_sender_id: 'user-1',
              human_sender_path: '/users/user-1',
            },
          };
        }
        if (path === replyPath) {
          throw new Error('synthetic-tool-processing-error');
        }
        if (path === '/agents/sample-assistant') {
          return {
            path,
            node_type: 'raisin:Agent',
            properties: {
              user_id: 'agent:sample-assistant',
              display_name: 'Sample Assistant',
              provider: 'groq',
              model: 'llama-3.3-70b-versatile',
            },
          };
        }
        return null;
      },
      async updateProperty() {},
      async create() {
        throw new Error('Unexpected create in safety-net test');
      },
      async getChildren() {
        return [];
      },
    },
    sql: {
      async query(queryText) {
        if (queryText.includes("node_type = 'raisin:User'")) {
          return [{ path: '/users/user-1' }];
        }
        return [];
      },
    },
    events: {
      async emit() {},
    },
    ai: {
      async complete() {
        throw new Error('Unexpected model call in safety-net test');
      },
    },
  };

  await assert.rejects(
    () => handleUserMessage({ workspace: 'ai', event: { node_path: messagePath } }),
    /synthetic-tool-processing-error/,
  );
});

test('queues inbound user message when another assistant turn is in-flight', async () => {
  const messagePath = '/agents/sample-assistant/inbox/chats/chat-2/msg-2';
  const chatPath = '/agents/sample-assistant/inbox/chats/chat-2';
  const replyPath = `${chatPath}/reply-to-msg-2`;
  const updates = [];
  let modelCalled = false;

  globalThis.raisin = {
    nodes: {
      async get(_workspace, path) {
        if (path === messagePath) {
          return {
            path,
            node_type: 'raisin:Message',
            properties: {
              role: 'user',
              message_type: 'chat',
              status: 'delivered',
              content: 'second message while busy',
            },
          };
        }
        if (path === chatPath) {
          return {
            path,
            node_type: 'raisin:Conversation',
            properties: {
              agent_ref: '/agents/sample-assistant',
              participants: ['user-2'],
              human_sender_id: 'user-2',
              human_sender_path: '/users/user-2',
            },
          };
        }
        if (path === replyPath) {
          return null;
        }
        // The in-flight guard reads the latest assistant message's phase via
        // nodes.get; return a still-running prior turn.
        if (path === '/agents/sample-assistant/inbox/chats/chat-2/reply-to-msg-1') {
          return {
            path,
            node_type: 'raisin:Message',
            properties: { role: 'assistant', dispatch_phase: 'awaiting_results' },
          };
        }
        if (path === '/agents/sample-assistant') {
          return {
            path,
            node_type: 'raisin:Agent',
            properties: {
              provider: 'groq',
              model: 'llama-3.3-70b-versatile',
              user_id: 'agent:sample-assistant',
              display_name: 'Sample Assistant',
            },
          };
        }
        return null;
      },
      async updateProperty(_workspace, path, key, value) {
        updates.push({ path, key, value });
      },
      async create() {
        throw new Error('Unexpected create in queueing test');
      },
      async getChildren() {
        return [];
      },
    },
    sql: {
      async query(queryText) {
        // findInFlightAssistantTurn: latest assistant message in the conversation.
        if (
          queryText.includes('CHILD_OF($1)') &&
          queryText.includes("'assistant'") &&
          queryText.includes('ORDER BY created_at DESC') &&
          queryText.includes('LIMIT 1')
        ) {
          return [{ path: '/agents/sample-assistant/inbox/chats/chat-2/reply-to-msg-1' }];
        }
        return [];
      },
    },
    events: {
      async emit() {},
    },
    ai: {
      async complete() {
        modelCalled = true;
        throw new Error('Model call must not happen while queued');
      },
    },
  };

  await handleUserMessage({ workspace: 'ai', event: { node_path: messagePath } });

  assert.equal(modelCalled, false);
  assert.ok(
    updates.some((u) => u.path === messagePath && u.key === 'orchestration_queue_state' && u.value === 'queued'),
    'expected message to be marked queued',
  );
});

test('findInFlightAssistantTurn: latest non-terminal assistant message is in-flight', async () => {
  globalThis.raisin = {
    sql: { async query() { return [{ path: '/c/reply-1' }]; } },
    nodes: {
      async get(_workspace, path) {
        return { path, node_type: 'raisin:Message', properties: { role: 'assistant', dispatch_phase: 'awaiting_results' } };
      },
    },
  };
  const result = await findInFlightAssistantTurn('ai', '/c');
  assert.deepEqual(result, { path: '/c/reply-1' });
});

test('findInFlightAssistantTurn: terminal latest message is NOT in-flight (stranded earlier message ignored)', async () => {
  // Regression for the "stuck after a tool is used" bug: the original
  // tool-dispatching message stays at 'ready_for_model', but the query only
  // returns the latest message (the terminal continuation), so the guard must
  // report no in-flight turn and let the next user message through.
  globalThis.raisin = {
    sql: { async query() { return [{ path: '/c/continue-1-reply-1' }]; } },
    nodes: {
      async get(_workspace, path) {
        return { path, node_type: 'raisin:Message', properties: { role: 'assistant', dispatch_phase: 'terminal' } };
      },
    },
  };
  const result = await findInFlightAssistantTurn('ai', '/c');
  assert.equal(result, null);
});

test('findInFlightAssistantTurn: no assistant messages is NOT in-flight', async () => {
  globalThis.raisin = {
    sql: { async query() { return []; } },
    nodes: { async get() { return null; } },
  };
  const result = await findInFlightAssistantTurn('ai', '/c');
  assert.equal(result, null);
});
