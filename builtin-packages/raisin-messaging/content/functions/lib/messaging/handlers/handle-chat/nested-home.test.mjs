/**
 * User-to-user delivery from nested homes (`/users/internal/<slug>`): the
 * sender is derived from the outbox path when the message carries no
 * sender_path, and the sent copy lands in the nested home.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from '../../../../../../../ai-tools/tests/support/fake-raisin.mjs';

const { handle_chat, entityPathFromOutboxPath } = await import('./index.js');

const AC = 'raisin:access_control';

function world() {
  const f = fakeRaisin();
  // The fake SQL engine has no GRAPH_TABLE: nobody is blocked.
  const query = f.api.sql.query;
  f.api.sql.query = async (sql, params) => (/GRAPH_TABLE/.test(sql) ? [] : query(sql, params));
  f.api.nodes.getById = async (ws, id) =>
    [...f.nodes.values()].find((n) => n.workspace === ws && n.id === id) || null;
  f.put(AC, '/users/internal/alice', {
    id: 'node-alice', node_type: 'raisin:User',
    properties: { user_id: 'idn-alice', display_name: 'Alice' },
  });
  f.put(AC, '/users/internal/bob', {
    id: 'node-bob', node_type: 'raisin:User',
    properties: { user_id: 'idn-bob', display_name: 'Bob' },
  });
  return f;
}

function message(props) {
  return {
    id: 'm-1',
    path: '/users/internal/alice/outbox/msg-1',
    node_type: 'raisin:Message',
    properties: {
      message_type: 'chat', status: 'pending',
      body: { content: 'hi', message_text: 'hi', thread_id: 'conv-1' },
      ...props,
    },
  };
}

async function send(f, node) {
  f.put(AC, node.path, node);
  return handle_chat({ node, workspace: AC });
}

test('entityPathFromOutboxPath keeps nested homes whole', () => {
  assert.equal(entityPathFromOutboxPath('/users/internal/alice/outbox/msg-1'), '/users/internal/alice');
  assert.equal(entityPathFromOutboxPath('/users/alice/outbox/msg-1'), '/users/alice');
  assert.equal(entityPathFromOutboxPath('/agents/site/outbox/m'), '/agents/site');
  assert.equal(entityPathFromOutboxPath('/visitors/abc/outbox/m'), '/visitors/abc');
  assert.equal(entityPathFromOutboxPath('/users/alice/inbox/m'), null);
  assert.equal(entityPathFromOutboxPath('/users/outbox/m'), null);
  assert.equal(entityPathFromOutboxPath('/other/a/outbox/m'), null);
  assert.equal(entityPathFromOutboxPath(''), null);
  assert.equal(entityPathFromOutboxPath(undefined), null);
});

test('a message without sender_path is delivered from a nested home', async () => {
  const f = world();
  await send(f, message({ recipient_path: '/users/internal/bob' }));
  const conv = f.nodes.get(`${AC}\u0000/users/internal/bob/inbox/chats/conv-1`);
  assert.ok(conv, 'conversation in the recipient home');
  assert.deepEqual(conv.properties.participants, ['node-alice', 'node-bob']);
  const delivered = f.nodes.get(`${AC}\u0000/users/internal/bob/inbox/chats/conv-1/msg-m-1`);
  assert.equal(delivered.properties.sender_id, 'node-alice');
  assert.equal(delivered.properties.sender_path, undefined, 'a person reads no home path');
  // Legacy (slug) homes have no key.
  assert.equal(delivered.properties.sender_key, null);
  assert.deepEqual(conv.properties.participant_keys, [null, null]);
  assert.ok(f.nodes.get(`${AC}\u0000/users/internal/alice/inbox/chats/conv-1/msg-m-1`));
  assert.ok(f.nodes.get(`${AC}\u0000/users/internal/alice/sent/msg-1`), 'sent copy in the nested home');
});

test('the recipient may be named by node id or by a path inside its home', async () => {
  const f = world();
  await send(f, message({ recipient_id: 'node-bob' }));
  assert.ok(f.nodes.get(`${AC}\u0000/users/internal/bob/inbox/chats/conv-1`));
  const g = world();
  await send(g, message({ recipient_path: '/users/internal/bob/outbox/x' }));
  assert.ok(g.nodes.get(`${AC}\u0000/users/internal/bob/inbox/chats/conv-1`));
});

test('a non-user node at the recipient path is not a recipient', async () => {
  const f = world();
  f.put(AC, '/users/internal/folder', { node_type: 'raisin:Folder', properties: {} });
  await assert.rejects(send(f, message({ recipient_path: '/users/internal/folder' })), /Recipient not found/);
});

test('a non-string id does not crash resolution', async () => {
  const f = world();
  await assert.rejects(send(f, message({ recipient_id: 42 })), /Recipient not found/);
});

// ─── Keyed homes ────────────────────────────────────────────────────────────

const KEY_A = 'u_' + 'A'.repeat(22);
const KEY_B = 'u_' + 'b1'.repeat(11);
const KEY_X = 'u_' + 'Z'.repeat(22);

function keyed() {
  const f = world();
  f.put(AC, `/users/internal/${KEY_A}`, {
    id: 'node-ka', node_type: 'raisin:User', properties: { user_id: 'idn-ka', display_name: 'Ka' },
  });
  f.put(AC, `/users/internal/${KEY_B}`, {
    id: 'node-kb', node_type: 'raisin:User', properties: { user_id: 'idn-kb', display_name: 'Kb' },
  });
  return f;
}

function keyedMessage(props) {
  return {
    id: 'm-k',
    path: `/users/internal/${KEY_A}/outbox/msg-k`,
    node_type: 'raisin:Message',
    properties: {
      message_type: 'chat', status: 'pending',
      body: { content: 'hi', message_text: 'hi', thread_id: 'conv-k' },
      ...props,
    },
  };
}

/** Every string anywhere in `value` (deep). */
function strings(value, out = []) {
  if (typeof value === 'string') out.push(value);
  else if (value && typeof value === 'object') for (const v of Object.values(value)) strings(v, out);
  return out;
}

test('keys come from the resolved homes; the copies a person reads hold no home path', async () => {
  const f = keyed();
  await send(f, keyedMessage({
    recipient_key: KEY_B,
    sender_key: KEY_X,
    participant_keys: [KEY_X],
    body: { content: 'hi', thread_id: 'conv-k', sender_key: KEY_X, sender_path: `/users/internal/${KEY_A}` },
  }));
  const bobConv = f.nodes.get(`${AC}\u0000/users/internal/${KEY_B}/inbox/chats/conv-k`);
  const bobMsg = f.nodes.get(`${AC}\u0000/users/internal/${KEY_B}/inbox/chats/conv-k/msg-m-k`);
  const aliceConv = f.nodes.get(`${AC}\u0000/users/internal/${KEY_A}/inbox/chats/conv-k`);
  const aliceMsg = f.nodes.get(`${AC}\u0000/users/internal/${KEY_A}/inbox/chats/conv-k/msg-m-k`);
  for (const n of [bobConv, bobMsg, aliceConv, aliceMsg]) {
    assert.ok(n, 'copy exists');
    const paths = strings(n.properties).filter((v) => v.includes('/users/'));
    assert.deepEqual(paths, [], `no home path in ${n.path}`);
    assert.ok(!strings(n.properties).includes(KEY_X), 'a client key never lands');
  }
  assert.equal(bobMsg.properties.sender_key, KEY_A);
  assert.equal(bobMsg.properties.recipient_key, KEY_B);
  assert.equal(bobMsg.properties.sender_id, 'node-ka');
  assert.deepEqual(bobConv.properties.participants, ['node-ka', 'node-kb']);
  assert.deepEqual(bobConv.properties.participant_keys, [KEY_A, KEY_B]);
  assert.equal(bobConv.properties.last_message.sender_key, KEY_A);
});

test('a recipient named by its key in recipient_id resolves; an unknown key does not', async () => {
  const f = keyed();
  await send(f, keyedMessage({ recipient_id: KEY_B }));
  assert.ok(f.nodes.get(`${AC}\u0000/users/internal/${KEY_B}/inbox/chats/conv-k`));
  const g = keyed();
  await assert.rejects(send(g, keyedMessage({ recipient_key: KEY_X })), /Recipient not found/);
});

test('a sender_path in the message cannot speak for another home', async () => {
  const f = keyed();
  await assert.rejects(send(f, keyedMessage({
    recipient_key: KEY_B,
    body: { content: 'hi', thread_id: 'conv-s', sender_path: `/users/internal/${KEY_B}` },
  })), /does not match the outbox/);
  assert.equal(f.nodes.get(`${AC}\u0000/users/internal/${KEY_B}/inbox/chats/conv-s`), undefined);
});
