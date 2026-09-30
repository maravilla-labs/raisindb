/**
 * Delivery for anonymous VISITORS (/visitors/<key>, raisin:VisitorSession).
 *
 * The pipeline treats a visitor like a user whose only possible correspondent
 * is an agent that allows anonymous chat: the message reaches the agent, the
 * reply reaches the visitor's conversation, and nothing crosses between
 * conversations or sessions.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from '../../../../../../../ai-tools/tests/support/fake-raisin.mjs';

const { handle_chat } = await import('./index.js');

const KEY_A = 'a'.repeat(32);
const KEY_B = 'b'.repeat(32);
const AC = 'raisin:access_control';

function world({ allowAnonymous = true } = {}) {
  const f = fakeRaisin();
  f.put('functions', '/agents/site', {
    node_type: 'raisin:AIAgent',
    properties: allowAnonymous ? { anonymous: { enabled: true } } : {},
  });
  for (const key of [KEY_A, KEY_B]) {
    f.put(AC, `/visitors/${key}`, {
      node_type: 'raisin:VisitorSession',
      properties: { agent_path: '/agents/site' },
    });
  }
  f.put(AC, '/users/internal/alice', {
    node_type: 'raisin:User',
    properties: { user_id: 'alice', display_name: 'Alice' },
  });
  return f;
}

function visitorMessage(key, conversationId, text = 'hello', recipient = 'agent:site') {
  return {
    id: `m-${key.slice(0, 4)}-${conversationId}`,
    path: `/visitors/${key}/outbox/msg-1`,
    node_type: 'raisin:Message',
    properties: {
      role: 'user', message_type: 'chat', status: 'pending',
      sender_id: `visitor:${key}`, sender_path: `/visitors/${key}`,
      recipient_id: recipient,
      body: { content: text, message_text: text, thread_id: conversationId },
      conversation_id: conversationId,
    },
  };
}

async function send(f, node) {
  f.put(AC, node.path, node);
  return handle_chat({ node, workspace: AC });
}

test('a visitor message reaches the agent, and the conversation lands in the visitor home', async () => {
  const f = world();
  await send(f, visitorMessage(KEY_A, 'vchat-1'));
  const agentConv = f.nodes.get(`ai\u0000/agents/site/inbox/chats/vchat-1`);
  assert.ok(agentConv, 'the agent received the conversation');
  assert.equal(agentConv.properties.human_sender_id, `visitor:${KEY_A}`);
  assert.equal(agentConv.properties.human_sender_path, `/visitors/${KEY_A}`);
  assert.ok(f.nodes.get(`ai\u0000/agents/site/inbox/chats/vchat-1/msg-m-aaaa-vchat-1`));
  assert.ok(f.nodes.get(`${AC}\u0000/visitors/${KEY_A}/inbox/chats/vchat-1`), 'the visitor side');
  assert.ok(![...f.nodes.keys()].some((k) => k.includes('/notifications/')), 'visitors get no notifications');
});

test('an agent without the flag refuses visitors', async () => {
  const f = world({ allowAnonymous: false });
  await assert.rejects(send(f, visitorMessage(KEY_A, 'vchat-1')), /does not accept anonymous/);
  assert.equal(f.nodes.get(`ai\u0000/agents/site/inbox/chats/vchat-1`), undefined);
});

test('a visitor cannot message a user', async () => {
  const f = world();
  await assert.rejects(
    send(f, visitorMessage(KEY_A, 'vchat-1', 'hi', '/users/internal/alice')),
    /only chat with agents/,
  );
});

test('a message claiming another visitor\'s id from this home is not delivered', async () => {
  const f = world();
  const forged = visitorMessage(KEY_A, 'vchat-1');
  forged.properties.sender_id = `visitor:${KEY_B}`;
  await assert.rejects(send(f, forged), /Sender not found/);
});

test('the agent\'s reply reaches the visitor\'s conversation', async () => {
  const f = world();
  await send(f, visitorMessage(KEY_A, 'vchat-1'));
  const reply = {
    id: 'r1', path: '/agents/site/outbox/msg-r1', node_type: 'raisin:Message',
    properties: {
      role: 'assistant', message_type: 'chat', status: 'pending',
      sender_id: 'agent:site', sender_path: '/agents/site',
      recipient_id: `visitor:${KEY_A}`, recipient_path: `/visitors/${KEY_A}`,
      body: { content: 'hi there', message_text: 'hi there', thread_id: 'vchat-1' },
      conversation_id: 'vchat-1',
    },
  };
  f.put('ai', reply.path, reply);
  await handle_chat({ node: reply, workspace: 'ai' });
  const delivered = f.nodes.get(`${AC}\u0000/visitors/${KEY_A}/inbox/chats/vchat-1/msg-r1`);
  assert.equal(delivered?.properties.body.content, 'hi there');
  assert.equal(f.nodes.get(`${AC}\u0000/visitors/${KEY_B}/inbox/chats/vchat-1`), undefined);
});

/**
 * The cross-conversation mix-up made structural: a second party reusing a
 * thread id cannot join (and redirect the replies of) someone else's
 * conversation with the agent.
 */
test('another visitor reusing a conversation id is refused', async () => {
  const f = world();
  await send(f, visitorMessage(KEY_A, 'vchat-1'));
  const other = visitorMessage(KEY_B, 'vchat-1');
  other.path = `/visitors/${KEY_B}/outbox/msg-2`;
  await assert.rejects(send(f, other), /belongs to another participant/);
  const agentConv = f.nodes.get(`ai\u0000/agents/site/inbox/chats/vchat-1`);
  assert.equal(agentConv.properties.human_sender_id, `visitor:${KEY_A}`, 'still A\'s');
});

test('a session bound to one agent cannot talk to another', async () => {
  const f = world();
  f.put('functions', '/agents/other', { node_type: 'raisin:AIAgent', properties: { allow_anonymous: true } });
  await assert.rejects(
    send(f, visitorMessage(KEY_A, 'vchat-9', 'hi', 'agent:other')),
    /belongs to another agent/,
  );
});

test('visitor paths and ids with traversal payloads name no session', async () => {
  const f = world();
  for (const [path, id] of [
    ['/visitors/../users/internal/alice', `visitor:${KEY_A}`],
    [`/visitors/${KEY_A}/../${KEY_B}`, null],
    ['/visitors/%2e%2e/users', null],
    [null, 'visitor:../../users/internal/alice'],
    [null, `visitor:${KEY_A}/../${KEY_B}`],
  ]) {
    const m = visitorMessage(KEY_A, 'vchat-9');
    m.properties.sender_path = path;
    m.properties.sender_id = id;
    await assert.rejects(send(f, m), /Sender not found/, JSON.stringify([path, id]));
  }
});
