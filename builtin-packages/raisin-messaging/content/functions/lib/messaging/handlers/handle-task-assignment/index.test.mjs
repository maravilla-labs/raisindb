/**
 * Task assignment through the outbox: to one person (held to task_permissions) or to a whole circle
 * (held to the circle's tasks policy). The sender is the outbox's home; the creation itself is
 * raisin.tasks.create, recorded here as the request it received.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from '../../../../../../../ai-tools/tests/support/fake-raisin.mjs';

const { handle_task_assignment, assignmentIdFor } = await import('./index.js');
const { circle_sync } = await import('../../circle-sync/index.js');

const AC = 'raisin:access_control';
const KEYS = {
  ana: 'u_AAAAAAAAAAAAAAAAAAAAAA',
  ben: 'u_BBBBBBBBBBBBBBBBBBBBBB',
  teacher: 'u_TTTTTTTTTTTTTTTTTTTTTT',
  mallory: 'u_MMMMMMMMMMMMMMMMMMMMMM',
};
const home = (who) => `/users/internal/${KEYS[who]}`;

function world({ messaging, config, blocks = [] } = {}) {
  const f = fakeRaisin();
  const edges = [];
  const query = f.api.sql.query;
  f.api.sql.query = async (sql, params) => {
    const n = /NEIGHBORS\(\$1, '(IN|OUT)', '(\w+)'\)/.exec(sql);
    if (n) {
      const [, dir, type] = n;
      return edges
        .filter((e) => e.type === type && (dir === 'IN' ? e.to === params[0] : e.from === params[0]))
        .map((e) => {
          const other = [...f.nodes.values()].find((x) => x.id === (dir === 'IN' ? e.from : e.to));
          return { id: other.id, path: other.path, node_type: other.node_type };
        });
    }
    if (/BLOCKS/.test(sql)) {
      const blocker = /blocker\.id = '([^']+)'/.exec(sql)?.[1];
      const blockee = /blockee\.id = '([^']+)'/.exec(sql)?.[1];
      return blocks.some(([a, b]) => a === blocker && b === blockee) ? [{ id: blockee }] : [];
    }
    if (/GRAPH_TABLE/.test(sql)) return [];
    return query(sql, params);
  };
  f.api.nodes.getById = async (ws, id) => [...f.nodes.values()].find((x) => x.workspace === ws && x.id === id) || null;
  const created = [];
  f.api.tasks = {
    async create(request) {
      created.push(request);
      if (request.assignee.startsWith('/circles/')) {
        const paths = edges.filter((e) => e.type === 'MEMBER_OF' && e.to === 'circle-piano')
          .map((e) => `${[...f.nodes.values()].find((x) => x.id === e.from).path}/inbox/${request.assignment_id}`);
        return { assignment_id: request.assignment_id, task_paths: paths, created: true };
      }
      const path = `${request.assignee}/inbox/task-${created.length}`;
      f.put(AC, path, { node_type: 'raisin:InboxTask', properties: { ...request, status: 'pending' } });
      return { task_id: `t-${created.length}`, task_path: path };
    },
  };
  for (const who of Object.keys(KEYS)) {
    f.put(AC, home(who), { id: `node-${who}`, node_type: 'raisin:User', properties: { user_id: `idn-${who}`, display_name: who } });
  }
  f.put(AC, '/circles/piano-a', {
    id: 'circle-piano', node_type: 'raisin:EntityCircle',
    properties: { name: 'Piano A', ...(messaging ? { messaging } : {}) },
  });
  if (config) f.put(AC, '/config/messaging', { node_type: 'raisin:MessagingConfig', properties: config });
  edges.push(
    { from: 'node-ana', to: 'circle-piano', type: 'MEMBER_OF' },
    { from: 'node-ben', to: 'circle-piano', type: 'MEMBER_OF' },
    { from: 'node-teacher', to: 'circle-piano', type: 'MODERATES' },
  );
  return { f, created, edges };
}

let seq = 0;
function outbox(f, who, body, extra = {}) {
  return f.put(AC, `${home(who)}/outbox/m-${++seq}`, {
    id: `out-${seq}`, node_type: 'raisin:Message',
    properties: { message_type: 'task_assignment', status: 'pending', body, ...extra },
  });
}

test('a moderator assigns a task to the whole circle', async () => {
  const { f, created } = world();
  const node = outbox(f, 'teacher', {
    assignee: 'circle:circle-piano', task_type: 'bring_item', title: 'Bring toilet paper on Saturday',
    completion: 'any', description: 'One pack is enough.',
  });
  const result = await handle_task_assignment({ node });
  assert.equal(result.success, true);
  assert.equal(node.properties.status, 'delivered');
  assert.equal(node.properties.assignment_id, assignmentIdFor('out-' + seq));

  assert.equal(created.length, 1);
  const [request] = created;
  assert.equal(request.assignee, '/circles/piano-a');
  assert.equal(request.task_type, 'bring_item');
  assert.equal(request.completion, 'any');
  assert.equal(request.conversation_id, 'grp-circle-piano', 'shown as a card in the group chat');
  assert.deepEqual(request.assigned_by, { key: KEYS.teacher, display_name: 'teacher' });
  assert.equal(request.assignment_id, node.properties.assignment_id, 'a retry finds the same record');

  const notes = [...f.nodes.values()].filter((n) => n.node_type === 'raisin:Notification');
  assert.deepEqual(notes.map((n) => n.path.split('/inbox/')[0]).sort(), [home('ana'), home('ben')].sort());
  assert.match(notes[0].properties.title, /Piano A: Bring toilet paper/);
});

test('by default members may not assign tasks to the whole circle', async () => {
  const { f, created } = world();
  const node = outbox(f, 'ana', { assignee: 'circle:circle-piano', task_type: 'bring_item', title: 'Bring cake' });
  await handle_task_assignment({ node });
  assert.equal(node.properties.status, 'failed');
  assert.equal(node.properties.failure_reason, 'not_allowed');
  assert.equal(created.length, 0);
});

test('a circle that lets members assign tasks', async () => {
  const { f, created } = world({ messaging: { tasks: 'members' } });
  const node = outbox(f, 'ana', { assignee: '/circles/piano-a', task_type: 'bring_item', title: 'Bring cake', completion: 'quorum', required: 2 });
  await handle_task_assignment({ node });
  assert.equal(node.properties.status, 'delivered');
  assert.equal(created[0].completion, 'quorum');
  assert.equal(created[0].required, 2);
});

test('an outsider cannot assign to a circle, and tasks can be switched off', async () => {
  const { f, created } = world();
  const outsider = outbox(f, 'mallory', { assignee: 'circle:circle-piano', task_type: 'action', title: 'x' });
  await handle_task_assignment({ node: outsider });
  assert.equal(outsider.properties.failure_reason, 'not_a_member');

  const closed = world({ messaging: { tasks: 'off' } });
  const node = outbox(closed.f, 'teacher', { assignee: 'circle:circle-piano', task_type: 'action', title: 'x' });
  await handle_task_assignment({ node });
  assert.equal(node.properties.failure_reason, 'tasks_closed');
  assert.equal(created.length + closed.created.length, 0);
});

test('a person assigns a task to another person', async () => {
  const { f, created } = world();
  const node = outbox(f, 'teacher', {
    assignee: KEYS.ana, task_type: 'sign_contract', title: 'Sign the course agreement',
    data: { document: '/contracts/piano-a' }, priority: 4, due_in_seconds: 86400,
  });
  const result = await handle_task_assignment({ node });
  assert.equal(result.success, true);
  assert.equal(created[0].assignee, home('ana'));
  assert.equal(created[0].task_type, 'sign_contract');
  assert.deepEqual(created[0].data, { document: '/contracts/piano-a' });
  assert.equal(created[0].source_message_id, node.id);
  assert.equal(node.properties.status, 'delivered');
});

test('a retried trigger does not assign the same task twice', async () => {
  const { f, created } = world();
  const node = outbox(f, 'teacher', { assignee: KEYS.ben, task_type: 'confirm_attendance', title: 'Coming on Friday?' });
  await handle_task_assignment({ node });
  node.properties.status = 'pending';
  await handle_task_assignment({ node });
  assert.equal(created.length, 1);
});

test('task_permissions and blocks apply between people', async () => {
  const restricted = world({ config: { task_permissions: { mode: 'any_of', rules: [{ type: 'sender_has_role', role: 'staff' }] } } });
  const node = outbox(restricted.f, 'ana', { assignee: KEYS.ben, task_type: 'action', title: 'Do my homework' });
  await handle_task_assignment({ node });
  assert.equal(node.properties.failure_reason, 'not_allowed');

  const blocked = world({ blocks: [['node-ben', 'node-mallory']] });
  const spam = outbox(blocked.f, 'mallory', { assignee: KEYS.ben, task_type: 'action', title: 'Click this' });
  await handle_task_assignment({ node: spam });
  assert.equal(spam.properties.failure_reason, 'not_allowed');
  assert.equal(restricted.created.length + blocked.created.length, 0);
});

test('the sender is the outbox, never a field', async () => {
  const { f, created } = world();
  const node = outbox(f, 'mallory', { assignee: 'circle:circle-piano', task_type: 'action', title: 'x', sender_key: KEYS.teacher });
  await handle_task_assignment({ node });
  assert.equal(node.properties.failure_reason, 'sender_mismatch');
  assert.equal(created.length, 0);
});

test('malformed tasks and unknown assignees are refused', async () => {
  const { f } = world();
  for (const [body, reason] of [
    [{ assignee: KEYS.ana, task_type: 'Not A Slug', title: 'x' }, 'bad_task'],
    [{ assignee: KEYS.ana, task_type: 'action', title: '' }, 'bad_task'],
    [{ assignee: 'u_ZZZZZZZZZZZZZZZZZZZZZZ', task_type: 'action', title: 'x' }, 'unknown_assignee'],
    [{ assignee: 'circle:nope', task_type: 'action', title: 'x' }, 'unknown_circle'],
    [{ assignee: KEYS.ana, task_type: 'action', title: 'x', conversation_id: 'conv-1', attachments: [{ 'raisin:ref': '/messages/conv-2/x.pdf', 'raisin:workspace': 'assets' }] }, 'not_this_conversation'],
  ]) {
    const node = outbox(f, 'teacher', body);
    await handle_task_assignment({ node });
    assert.equal(node.properties.failure_reason, reason, JSON.stringify(body));
  }
});

test('circle-sync gives new members the conversation and marks those who left', async () => {
  const { f, edges } = world();
  let out = await circle_sync({ circle: '/circles/piano-a' });
  assert.deepEqual(out, { conversation_id: 'grp-circle-piano', members: 3, created: 3, left: 0 });
  const conv = (who) => f.nodes.get(`${AC}\u0000${home(who)}/inbox/chats/grp-circle-piano`);
  assert.equal(conv('ana').properties.membership, 'member');
  assert.equal(conv('ana').properties.member_count, 2);
  assert.equal(conv('ana').properties.unread_count, 0);

  edges.splice(edges.findIndex((e) => e.from === 'node-ben'), 1);
  out = await circle_sync({ circle: 'circle:circle-piano' });
  assert.equal(out.left, 1);
  assert.equal(conv('ben').properties.membership, 'left');
  assert.equal(conv('ana').properties.member_count, 1);
  await assert.rejects(circle_sync({ circle: '/circles/none' }), /unknown_circle/);
});

test('a task names only a conversation the sender has with the assignee, and is held to the chat limits', async () => {
  const { f, created } = world({ config: { max_message_length: 40, rate_limit_messages_per_minute: 50 } });
  const foreign = outbox(f, 'mallory', { assignee: KEYS.ana, task_type: 'bring_item', title: 'x', conversation_id: 'grp-circle-piano',
    attachments: [{ 'raisin:ref': '/messages/grp-circle-piano/a.jpg', 'raisin:workspace': 'assets' }] });
  await handle_task_assignment({ node: foreign });
  assert.equal(foreign.properties.failure_reason, 'not_this_conversation');
  // Files from another conversation than the named one.
  f.put(AC, `${home('mallory')}/inbox/chats/conv-ma`, { node_type: 'raisin:Conversation', properties: { participants: ['node-ana', 'node-mallory'] } });
  const elsewhere = outbox(f, 'mallory', { assignee: KEYS.ana, task_type: 'bring_item', title: 'x', conversation_id: 'conv-ma',
    attachments: [{ 'raisin:ref': '/messages/conv-ab/a.jpg', 'raisin:workspace': 'assets' }] });
  await handle_task_assignment({ node: elsewhere });
  assert.equal(elsewhere.properties.failure_reason, 'foreign_attachment');
  const unknown = outbox(f, 'mallory', { assignee: KEYS.ana, task_type: 'bring_item', title: 'x', conversation_id: 'conv-none' });
  await handle_task_assignment({ node: unknown });
  assert.equal(unknown.properties.failure_reason, 'not_this_conversation');

  f.put(AC, `${home('ben')}/inbox/chats/conv-ab`, { node_type: 'raisin:Conversation', properties: { participants: ['node-ana', 'node-ben'] } });
  f.put(AC, `${home('ana')}/inbox/chats/conv-ab`, { node_type: 'raisin:Conversation', properties: { participants: ['node-ana', 'node-ben'] } });
  const ok = outbox(f, 'ben', { assignee: KEYS.ana, task_type: 'bring_item', title: 'Cake', conversation_id: 'conv-ab',
    attachments: [{ 'raisin:ref': '/messages/conv-ab/recipe.pdf', 'raisin:workspace': 'assets' }] });
  await handle_task_assignment({ node: ok });
  assert.equal(ok.properties.status, 'delivered', ok.properties.failure_reason);
  assert.equal(created.at(-1).conversation_id, 'conv-ab');

  const long = outbox(f, 'ben', { assignee: KEYS.ana, task_type: 'bring_item', title: 'Cake', description: 'x'.repeat(41) });
  await handle_task_assignment({ node: long });
  assert.equal(long.properties.failure_reason, 'too_long');
});
