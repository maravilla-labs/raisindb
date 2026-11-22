/**
 * Group chat: a message to a circle reaches every member and moderator, each in their own home, and only
 * the graph decides who belongs. Plus the limits a person's message is held to (length, rate) and the
 * attachment check, for one-to-one and group messages alike.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from '../../../../../../../ai-tools/tests/support/fake-raisin.mjs';

const { handle_chat } = await import('./index.js');

const AC = 'raisin:access_control';
const KEYS = {
  ana: 'u_AAAAAAAAAAAAAAAAAAAAAA',
  ben: 'u_BBBBBBBBBBBBBBBBBBBBBB',
  cara: 'u_CCCCCCCCCCCCCCCCCCCCCC',
  teacher: 'u_TTTTTTTTTTTTTTTTTTTTTT',
  mallory: 'u_MMMMMMMMMMMMMMMMMMMMMM',
};
const home = (who) => `/users/internal/${KEYS[who]}`;

/**
 * A world with a course circle: ana and ben are members, the teacher moderates, cara and mallory are
 * outside. `edges` is the relation index NEIGHBORS reads.
 */
function world({ messaging = undefined, config = undefined } = {}) {
  const f = fakeRaisin();
  const edges = [];
  const query = f.api.sql.query;
  f.api.sql.query = async (sql, params) => {
    const n = /NEIGHBORS\(\$1, '(IN|OUT)', '(\w+)'\)/.exec(sql);
    if (n) {
      const [, dir, type] = n;
      const hits = edges.filter((e) => e.type === type && (dir === 'IN' ? e.to === params[0] : e.from === params[0]));
      return hits.map((e) => {
        const other = [...f.nodes.values()].find((x) => x.id === (dir === 'IN' ? e.from : e.to));
        return { id: other.id, path: other.path, node_type: other.node_type };
      });
    }
    if (/GRAPH_TABLE/.test(sql)) return [];
    return query(sql, params);
  };
  f.api.nodes.getById = async (ws, id) =>
    [...f.nodes.values()].find((x) => x.workspace === ws && x.id === id) || null;
  for (const who of Object.keys(KEYS)) {
    f.put(AC, home(who), {
      id: `node-${who}`, node_type: 'raisin:User',
      properties: { user_id: `idn-${who}`, display_name: who[0].toUpperCase() + who.slice(1) },
    });
  }
  f.put(AC, '/circles/piano-a', {
    id: 'circle-piano', node_type: 'raisin:EntityCircle',
    properties: { name: 'Piano A', circle_type: 'course', ...(messaging ? { messaging } : {}) },
  });
  if (config) f.put(AC, '/config/messaging', { node_type: 'raisin:MessagingConfig', properties: config });
  edges.push(
    { from: 'node-ana', to: 'circle-piano', type: 'MEMBER_OF' },
    { from: 'node-ben', to: 'circle-piano', type: 'MEMBER_OF' },
    { from: 'node-teacher', to: 'circle-piano', type: 'MODERATES' },
  );
  return { f, edges };
}

let seq = 0;
function outbox(f, who, properties) {
  const name = `msg-${++seq}`;
  return f.put(AC, `${home(who)}/outbox/${name}`, {
    // The fake's own clock starts in the past; the rate limit counts by the real minute.
    id: `out-${seq}`, node_type: 'raisin:Message', created_at: new Date(Date.now() + seq).toISOString(),
    properties: { message_type: 'chat', status: 'pending', ...properties },
  });
}

const conv = (who) => `${home(who)}/inbox/chats/grp-circle-piano`;
const messagesIn = (f, path) => [...f.nodes.values()].filter((n) => n.path.startsWith(`${path}/msg-`));

test('a member posts to the circle: every member and moderator gets it in their own home', async () => {
  const { f } = world();
  const node = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'Who brings the snacks?' } });

  const result = await handle_chat({ node });
  assert.equal(result.success, true);
  assert.equal(result.delivered, 2, 'ben and the teacher');
  assert.equal(node.properties.status, 'delivered');

  for (const who of ['ana', 'ben', 'teacher']) {
    const c = f.nodes.get(`${AC}\u0000${conv(who)}`);
    assert.ok(c, `${who} has the conversation`);
    assert.equal(c.properties.conversation_type, 'group');
    assert.equal(c.properties.subject, 'Piano A');
    assert.deepEqual(c.properties.participants, ['circle:circle-piano']);
    assert.equal(c.properties.member_count, 2);
    assert.equal(c.properties.unread_count, who === 'ana' ? 0 : 1);
    assert.equal(c.properties.last_message.sender_display_name, 'Ana');
    const [m] = messagesIn(f, conv(who));
    assert.equal(m.properties.status, who === 'ana' ? 'sent' : 'delivered');
    assert.equal(m.properties.body.message_text, 'Who brings the snacks?');
    assert.equal(m.properties.sender_key, KEYS.ana);
    assert.equal(JSON.stringify(m.properties).includes('/users/internal/'), false, 'no home path in a copy');
  }
  for (const who of ['cara', 'mallory']) {
    assert.equal(f.nodes.get(`${AC}\u0000${conv(who)}`), undefined, `${who} is not in the circle`);
  }
  const notes = [...f.nodes.values()].filter((n) => n.node_type === 'raisin:Notification');
  assert.equal(notes.length, 2);
  assert.match(notes[0].properties.title, /Ana in Piano A/);
});

test('a second post counts unread up for the others and not for the sender', async () => {
  const { f } = world();
  await handle_chat({ node: outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'one' } }) });
  await handle_chat({ node: outbox(f, 'ben', { conversation_id: 'grp-circle-piano', body: { message_text: 'two' } }) });
  assert.equal(f.nodes.get(`${AC}\u0000${conv('ana')}`).properties.unread_count, 1);
  assert.equal(f.nodes.get(`${AC}\u0000${conv('ben')}`).properties.unread_count, 1);
  assert.equal(f.nodes.get(`${AC}\u0000${conv('teacher')}`).properties.unread_count, 2);
  assert.equal(messagesIn(f, conv('teacher')).length, 2);
});

test('someone outside the circle cannot post into it, whatever the message says', async () => {
  const { f } = world();
  const node = outbox(f, 'mallory', {
    recipient_id: 'circle:circle-piano', conversation_id: 'grp-circle-piano',
    body: { message_text: 'hello', members: [KEYS.mallory] },
  });
  const result = await handle_chat({ node });
  assert.equal(result.success, false);
  assert.equal(node.properties.status, 'failed');
  assert.equal(node.properties.failure_reason, 'not_a_member');
  assert.equal(f.nodes.get(`${AC}\u0000${conv('ana')}`), undefined);
});

test('an announcement circle: members read, only moderators post', async () => {
  const { f } = world({ messaging: { chat: 'moderators' } });
  const member = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'can I?' } });
  assert.equal((await handle_chat({ node: member })).success, false);
  assert.equal(member.properties.failure_reason, 'read_only');

  const teacher = outbox(f, 'teacher', { recipient_id: 'circle:circle-piano', body: { message_text: 'No class on Friday.' } });
  assert.equal((await handle_chat({ node: teacher })).success, true);
  assert.equal(messagesIn(f, conv('ana')).length, 1);
});

test('a closed circle takes no posts, an unknown one is refused', async () => {
  const { f } = world({ messaging: { chat: 'off' } });
  const closed = outbox(f, 'teacher', { recipient_id: 'circle:circle-piano', body: { message_text: 'x' } });
  await handle_chat({ node: closed });
  assert.equal(closed.properties.failure_reason, 'group_closed');

  const unknown = outbox(f, 'ana', { recipient_id: 'circle:nope', body: { message_text: 'x' } });
  await handle_chat({ node: unknown });
  assert.equal(unknown.properties.failure_reason, 'unknown_circle');
});

test('a circle above its member limit is refused rather than half delivered', async () => {
  const { f } = world({ messaging: { max_members: 2 } });
  const node = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'x' } });
  await handle_chat({ node });
  assert.equal(node.properties.failure_reason, 'group_too_large', 'two members and a moderator');
});

test('max_message_length is enforced', async () => {
  const { f } = world({ config: { max_message_length: 10 } });
  const node = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'far more than ten characters' } });
  await handle_chat({ node });
  assert.equal(node.properties.status, 'failed');
  assert.equal(node.properties.failure_reason, 'too_long');
});

test('rate_limit_messages_per_minute is enforced', async () => {
  const { f } = world({ config: { rate_limit_messages_per_minute: 2 } });
  const results = [];
  for (let i = 0; i < 3; i++) {
    const node = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: `m${i}` } });
    await handle_chat({ node });
    results.push(node.properties.failure_reason ?? node.properties.status);
  }
  assert.deepEqual(results, ['delivered', 'delivered', 'rate_limited']);
});

test('attachments: only references below the conversation folder are delivered', async () => {
  const { f } = world();
  const good = { 'raisin:ref': '/messages/grp-circle-piano/a1.jpg', 'raisin:workspace': 'assets', name: 'a1.jpg', mime_type: 'image/jpeg', size: 1200, extra: 'dropped' };
  const node = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'photo', attachments: [good] } });
  await handle_chat({ node });
  const [m] = messagesIn(f, conv('ben'));
  assert.deepEqual(m.properties.attachments, [
    { ref: { 'raisin:ref': '/messages/grp-circle-piano/a1.jpg', 'raisin:workspace': 'assets' }, name: 'a1.jpg', mime_type: 'image/jpeg', size: 1200 },
  ], 'the reference in its own object, so the engine keeps the name and type beside it');

  // The wrapped shape is accepted as input too.
  const wrapped = outbox(f, 'ben', { recipient_id: 'circle:circle-piano', body: { message_text: 'pdf', attachments: [
    { ref: { 'raisin:ref': '/messages/grp-circle-piano/b.pdf', 'raisin:workspace': 'assets' }, name: 'b.pdf', mime_type: 'application/pdf' },
  ] } });
  await handle_chat({ node: wrapped });
  assert.equal(wrapped.properties.status, 'delivered');

  for (const [bad, reason] of [
    [{ 'raisin:ref': '/messages/conv-other/x.jpg', 'raisin:workspace': 'assets' }, 'foreign_attachment'],
    [{ 'raisin:ref': '/secrets/x.pdf', 'raisin:workspace': 'assets' }, 'foreign_attachment'],
    [{ 'raisin:ref': '/messages/grp-circle-piano/x.jpg', 'raisin:workspace': 'raisin:access_control' }, 'bad_attachments'],
    ['nope', 'bad_attachments'],
  ]) {
    const n = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'x', attachments: [bad] } });
    await handle_chat({ node: n });
    assert.equal(n.properties.failure_reason, reason, JSON.stringify(bad));
  }
  const many = Array.from({ length: 11 }, (_, i) => ({ 'raisin:ref': `/messages/grp-circle-piano/${i}.jpg`, 'raisin:workspace': 'assets' }));
  const n = outbox(f, 'ana', { recipient_id: 'circle:circle-piano', body: { message_text: 'x', attachments: many } });
  await handle_chat({ node: n });
  assert.equal(n.properties.failure_reason, 'too_many_attachments');
});

test('one-to-one attachments are held to their conversation too', async () => {
  const { f } = world();
  // Files only from a conversation the sender already has: the first message of a new one carries none.
  const early = outbox(f, 'ana', {
    recipient_id: KEYS.ben, conversation_id: 'conv-ab',
    body: { message_text: 'doc', thread_id: 'conv-ab', attachments: [{ 'raisin:ref': '/messages/conv-ab/d.pdf', 'raisin:workspace': 'assets' }] },
  });
  await handle_chat({ node: early });
  assert.equal(early.properties.failure_reason, 'bad_attachments');
  const hello = outbox(f, 'ana', { recipient_id: KEYS.ben, conversation_id: 'conv-ab', body: { message_text: 'hi', thread_id: 'conv-ab' } });
  await handle_chat({ node: hello });
  assert.equal(hello.properties.status, 'delivered');

  const ok = outbox(f, 'ana', {
    recipient_id: KEYS.ben, conversation_id: 'conv-ab',
    body: { message_text: 'doc', thread_id: 'conv-ab', attachments: [{ 'raisin:ref': '/messages/conv-ab/d.pdf', 'raisin:workspace': 'assets' }] },
  });
  await handle_chat({ node: ok });
  assert.equal(ok.properties.status, 'delivered');
  const copy = [...f.nodes.values()].find((n) => n.path.startsWith(`${home('ben')}/inbox/chats/conv-ab/msg-`) && n.properties.attachments?.length);
  assert.equal(copy.properties.attachments[0].ref['raisin:ref'], '/messages/conv-ab/d.pdf');

  const bad = outbox(f, 'ana', {
    recipient_id: KEYS.ben, conversation_id: 'conv-ab',
    body: { message_text: 'doc', thread_id: 'conv-ab', attachments: [{ 'raisin:ref': '/messages/grp-circle-piano/x.jpg', 'raisin:workspace': 'assets' }] },
  });
  await handle_chat({ node: bad });
  assert.equal(bad.properties.failure_reason, 'foreign_attachment');
});

test("a one-to-one message cannot name another conversation's id", async () => {
  const { f } = world();
  // A group's conversation: mallory (outside) tries to plant it in cara's inbox, and with it a claim on its files.
  const group = outbox(f, 'mallory', { recipient_id: KEYS.cara, body: { message_text: 'x', thread_id: 'grp-circle-piano' } });
  await handle_chat({ node: group });
  // A grp- thread is taken as the circle itself, where an outsider is refused.
  assert.equal(group.properties.status, 'failed');
  assert.ok(![...f.nodes.values()].some((n) => n.path.startsWith(`${home('cara')}/inbox/chats/grp-`)));

  // Someone else's one-to-one: ana and ben talk in conv-ab; mallory can join neither copy.
  const ab = outbox(f, 'ana', { recipient_id: KEYS.ben, body: { message_text: 'hi', thread_id: 'conv-ab' } });
  await handle_chat({ node: ab });
  assert.equal(ab.properties.status, 'delivered');
  const toBen = outbox(f, 'mallory', { recipient_id: KEYS.ben, body: { message_text: 'x', thread_id: 'conv-ab' } });
  await handle_chat({ node: toBen });
  assert.equal(toBen.properties.failure_reason, 'not_this_conversation');
  const mine = outbox(f, 'ana', { recipient_id: KEYS.cara, body: { message_text: 'x', thread_id: 'conv-ab' } });
  await handle_chat({ node: mine });
  assert.equal(mine.properties.failure_reason, 'not_this_conversation', 'ana cannot pull cara into her conversation with ben');
  const benCopy = [...f.nodes.values()].find((n) => n.path === `${home('ben')}/inbox/chats/conv-ab`);
  assert.equal(benCopy.properties.participants.length, 2);

  // Two accounts of one person cannot adopt a third conversation's id that neither of them holds.
  const adopt = outbox(f, 'mallory', { recipient_id: KEYS.cara, body: { message_text: 'x', thread_id: 'conv-ab' } });
  await handle_chat({ node: adopt });
  assert.equal(adopt.properties.failure_reason, 'not_this_conversation');
  assert.equal([...f.nodes.values()].some((n) => n.path === `${home('cara')}/inbox/chats/conv-ab`), false);

  // A copy planted under conv-ab in someone else's inbox neither blocks ana and ben nor joins mallory.
  f.put(AC, `${home('mallory')}/inbox/chats/conv-ab`, { node_type: 'raisin:Conversation', properties: { conversation_id: 'conv-ab', participants: ['node-mallory', 'node-cara'] } });
  const still = outbox(f, 'ben', { recipient_id: KEYS.ana, body: { message_text: 'still here', thread_id: 'conv-ab' } });
  await handle_chat({ node: still });
  assert.equal(still.properties.status, 'delivered');
  const record = [...f.nodes.values()].find((n) => n.path === '/conversation-records/conv-ab');
  assert.deepEqual([...record.properties.participants].sort(), ['node-ana', 'node-ben']);

  // No thread named: a fresh conversation.
  const fresh = outbox(f, 'mallory', { recipient_id: KEYS.cara, body: { message_text: 'hello' } });
  await handle_chat({ node: fresh });
  assert.equal(fresh.properties.status, 'delivered');
});

test('a conversation begun before records existed cannot be claimed by others', async () => {
  const { f } = world();
  // ana and ben's old conversation: copies in their homes, no record.
  for (const who of ['ana', 'ben']) {
    f.put(AC, `${home(who)}/inbox/chats/conv-old`, { node_type: 'raisin:Conversation', properties: { conversation_id: 'conv-old', participants: ['node-ana', 'node-ben'] } });
  }
  const claim = outbox(f, 'mallory', { recipient_id: KEYS.cara, body: { message_text: 'x', thread_id: 'conv-old' } });
  await handle_chat({ node: claim });
  assert.equal(claim.properties.failure_reason, 'not_this_conversation');
  assert.equal([...f.nodes.values()].some((n) => n.path === '/conversation-records/conv-old'), false);
  // Its own parties carry on, and that registers it.
  const own = outbox(f, 'ana', { recipient_id: KEYS.ben, body: { message_text: 'still us', thread_id: 'conv-old' } });
  await handle_chat({ node: own });
  assert.equal(own.properties.status, 'delivered');
  assert.ok([...f.nodes.values()].some((n) => n.path === '/conversation-records/conv-old'));
});
