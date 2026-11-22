#!/usr/bin/env node
// Live check of group chat and group tasks (docs/plans/group-messaging-and-tasks.md) against a running
// server, as real people with their own tokens:
//
//   node scripts/group-messaging-e2e.mjs --server http://localhost:8097 --password 'Test12345678!'
//
// Creates a fresh repository (gm-e2e-<time>), four identity users, a circle with two members and a
// moderator, then: a member posts to the circle (both other people get it in their own inbox), an outsider
// is refused, the moderator assigns "bring toilet paper" to the circle (every member gets a task), the first
// answer closes it (the other task is cancelled and names who took it), a member reopening their own copy
// cannot answer again, and a member cannot read another member's inbox. Exits non-zero on the first
// broken expectation. Leaves the repository for inspection.

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => (a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]]] : acc), []),
);
const server = (args.server || 'http://localhost:8097').replace(/\/$/, '');
const password = args.password || process.env.RAISIN_ADMIN_PASSWORD || 'Test12345678!';
const tenant = args.tenant || 'default';
const repo = args.repo || `gm-e2e-${Date.now().toString(36)}`;
const AC = 'raisin:access_control';
const PW = 'Member12345!x';

let failed = 0;
const expect = (name, ok, extra = '') => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ' ' + extra}`);
  if (!ok) failed++;
  return ok;
};
const must = (name, ok, extra) => {
  if (!expect(name, ok, extra)) process.exit(1);
};
const call = async (method, path, { token, body } = {}) => {
  const res = await fetch(server + path, {
    method,
    headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(body ? { 'Content-Type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  let json;
  try { json = JSON.parse(text); } catch { json = text; }
  return { status: res.status, ok: res.ok, json };
};
const sql = async (token, query, params = []) => {
  const r = await call('POST', `/api/sql/${repo}`, { token, body: { sql: query, params } });
  if (!r.ok) throw new Error(`SQL ${r.status}: ${JSON.stringify(r.json)} for ${query}`);
  return r.json.rows ?? r.json;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(name, fn, ms = 20000) {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e.message;
    }
    await sleep(300);
  }
  must(name, false, `timed out (${JSON.stringify(last)})`);
}

// --- Admin, repository, people ------------------------------------------------
const auth = await call('POST', `/api/raisindb/sys/${tenant}/auth`, { body: { username: 'admin', password } });
must('admin login', auth.ok, JSON.stringify(auth.json));
const admin = auth.json.token;

const created = await call('POST', '/api/repositories', { token: admin, body: { repo_id: repo } });
must(`repository ${repo}`, created.ok, JSON.stringify(created.json));
// The builtin packages install concurrently and retry a lost folder race, so wait for the TRIGGERS
// (a message written before its trigger exists is never processed).
await until('builtin messaging installed', async () => {
  for (const path of ['lib/messaging/handlers/shared', 'triggers/raisin/messaging/process-chat', 'triggers/raisin/messaging/process-task-assignment']) {
    const r = await call('GET', `/api/repository/${repo}/main/head/functions/${path}`, { token: admin });
    if (!r.ok) return false;
  }
  return true;
}, 120000);

// Open self-registration for this test repository (it defaults to closed).
{
  const base = `/api/repository/${repo}/main/head/raisin:system`;
  const commit = { message: 'group messaging e2e: open sign-up', actor: 'group-messaging-e2e' };
  const existing = await call('GET', `${base}/config/repos/${repo}`, { token: admin });
  if (existing.ok && existing.json?.properties) {
    const put = await call('PUT', `${base}/config/repos/${repo}`, { token: admin, body: { properties: { ...existing.json.properties, allow_registration: true }, commit } });
    must('open sign-up', put.ok, JSON.stringify(put.json));
  } else {
    for (const [parent, name] of [[`${base}/`, 'config'], [`${base}/config`, 'repos']]) {
      await call('POST', parent, { token: admin, body: { name, node_type: 'raisin:Folder', properties: {}, commit } });
    }
    const post = await call('POST', `${base}/config/repos`, { token: admin, body: { name: repo, node_type: 'raisin:RepoAuthConfig', properties: { repo_id: repo, allow_registration: true }, commit } });
    must('open sign-up', post.ok, JSON.stringify(post.json));
  }
}

const people = {};
for (const [who, name] of [['ana', 'Ana'], ['ben', 'Ben'], ['teacher', 'Mrs. Field'], ['mallory', 'Mallory']]) {
  const email = `${who}-${repo}@example.com`;
  const reg = await call('POST', `/auth/${repo}/register`, { body: { email, password: PW, display_name: name } });
  must(`register ${who}`, reg.ok || /exists/i.test(JSON.stringify(reg.json)), JSON.stringify(reg.json));
  const login = await call('POST', `/auth/${repo}/login`, { body: { email, password: PW } });
  must(`login ${who}`, login.ok, JSON.stringify(login.json));
  const token = login.json.access_token;
  const me = await sql(token, `SELECT RAISIN_CURRENT_USER()->>'path'::String AS home, RAISIN_CURRENT_USER()->>'id'::String AS id`);
  const home = String(me[0].home).replace(/^\/raisin:access_control/, '');
  people[who] = { token, home, id: me[0].id, name };
}

// --- The circle ------------------------------------------------------------------
await sql(admin, `INSERT INTO '${AC}' (path, node_type, properties) VALUES ($1, 'raisin:EntityCircle', $2::jsonb)`,
  ['/circles/piano-a', JSON.stringify({ name: 'Piano A', circle_type: 'course' })]);
const circleId = (await sql(admin, `SELECT id FROM '${AC}' WHERE path = '/circles/piano-a'`))[0].id;
for (const [who, type] of [['ana', 'MEMBER_OF'], ['ben', 'MEMBER_OF'], ['teacher', 'MODERATES']]) {
  await sql(admin, `RELATE FROM path='${people[who].home}' IN WORKSPACE '${AC}' TO path='/circles/piano-a' IN WORKSPACE '${AC}' TYPE '${type}'`);
}
const roster = await sql(admin, `SELECT id, path, node_type FROM NEIGHBORS($1, 'IN', 'MEMBER_OF')`, [circleId]);
must('NEIGHBORS reads the members', roster.length === 2, JSON.stringify(roster));
const conv = `grp-${circleId}`;

// --- Group chat ------------------------------------------------------------------------
const post = async (who, properties) => {
  const name = `msg-${Math.random().toString(36).slice(2, 10)}`;
  const path = `${people[who].home}/outbox/${name}`;
  await sql(people[who].token, `INSERT INTO '${AC}' (path, node_type, properties) VALUES ($1, 'raisin:Message', $2::jsonb)`,
    [path, JSON.stringify({
      status: 'pending', created_at: new Date().toISOString(), sender_id: people[who].id,
      recipient_id: properties.recipient_id ?? properties.body?.assignee, ...properties,
    })]);
  return path;
};
const outboxStatus = async (who, path) => (await sql(people[who].token, `SELECT properties FROM '${AC}' WHERE path = $1`, [path]))[0]?.properties;

const p1 = await post('ana', { message_type: 'chat', recipient_id: `circle:${circleId}`, body: { message_text: 'Who brings the snacks?' } });
await until('ana\'s group message delivered', async () => (await outboxStatus('ana', p1))?.status === 'delivered');
for (const who of ['ben', 'teacher']) {
  const rows = await sql(people[who].token,
    `SELECT properties FROM '${AC}' WHERE CHILD_OF($1) AND node_type = 'raisin:Message'`, [`${people[who].home}/inbox/chats/${conv}`]);
  expect(`${who} has it in their own inbox`, rows.length === 1 && rows[0].properties.body?.message_text === 'Who brings the snacks?', JSON.stringify(rows));
  const c = await sql(people[who].token, `SELECT properties FROM '${AC}' WHERE path = $1`, [`${people[who].home}/inbox/chats/${conv}`]);
  expect(`${who}'s copy is a group conversation with one unread`, c[0]?.properties?.conversation_type === 'group' && c[0]?.properties?.unread_count === 1, JSON.stringify(c[0]?.properties));
}
const foreign = await sql(people.ben.token, `SELECT path FROM '${AC}' WHERE DESCENDANT_OF($1)`, [`${people.ana.home}/inbox`]);
expect('ben cannot read ana\'s inbox', foreign.length === 0, JSON.stringify(foreign));

const p2 = await post('mallory', { message_type: 'chat', recipient_id: `circle:${circleId}`, conversation_id: conv, body: { message_text: 'let me in' } });
const refused = await until('outsider refused', async () => {
  const s = await outboxStatus('mallory', p2);
  return s?.status === 'failed' ? s : null;
});
expect('… as not_a_member', refused.failure_reason === 'not_a_member', JSON.stringify(refused));

// --- Group task ---------------------------------------------------------------------------
const t1 = await post('teacher', { message_type: 'task_assignment', body: { assignee: `circle:${circleId}`, task_type: 'bring_item', title: 'Bring toilet paper on Saturday', completion: 'any' } });
const assigned = await until('task assigned', async () => {
  const s = await outboxStatus('teacher', t1);
  if (s?.status === 'failed') must('task assignment', false, JSON.stringify(s));
  return s?.status === 'delivered' ? s : null;
});
const tasksOf = async (who) => (await call('GET', `/api/inbox/${repo}`, { token: people[who].token })).json.tasks ?? [];
const anaTask = (await until('ana got the task', async () => (await tasksOf('ana')).find((t) => t.assignment_id === assigned.assignment_id)));
const benTask = (await until('ben got the task', async () => (await tasksOf('ben')).find((t) => t.assignment_id === assigned.assignment_id)));
expect('the task is in the group chat', anaTask.conversation_id === conv, anaTask.conversation_id);
expect('it names who assigned it', anaTask.assigned_by?.display_name === 'Mrs. Field', JSON.stringify(anaTask.assigned_by));
expect('no copy names another member\'s inbox', !JSON.stringify(anaTask).includes(people.ben.home));

const done = await call('POST', `/api/inbox/${repo}/tasks/${encodeURIComponent(anaTask.id)}/complete`, { token: people.ana.token, body: { response: { action: 'take' } } });
expect('ana takes it', done.ok, JSON.stringify(done.json));
const benAfter = await until('ben\'s task closed', async () => {
  const t = (await tasksOf('ben')).find((x) => x.id === benTask.id);
  return t?.status === 'cancelled' ? t : null;
});
expect('… as group_completed', benAfter.cancellation_reason === 'group_completed', JSON.stringify(benAfter));
expect('… naming who took it', JSON.stringify(benAfter.resolved_by_names) === JSON.stringify(['Ana']), JSON.stringify(benAfter.resolved_by_names));

// Ben reopens his own copy and answers: the record is closed.
await sql(people.ben.token, `UPDATE '${AC}' SET properties = jsonb_set(properties, '{status}', '"pending"'::jsonb) WHERE path = $1`, [benTask.path]);
const again = await call('POST', `/api/inbox/${repo}/tasks/${encodeURIComponent(benTask.id)}/complete`, { token: people.ben.token, body: { response: { action: 'take' } } });
expect('a reopened copy cannot answer a closed task', !again.ok, `${again.status} ${JSON.stringify(again.json)}`);
const record = await sql(admin, `SELECT properties FROM '${AC}' WHERE path = $1`, [`/task-assignments/${assigned.assignment_id}`]);
expect('the record is satisfied', record[0]?.properties?.status === 'satisfied', JSON.stringify(record[0]?.properties));
const recordAsMember = await sql(people.ana.token, `SELECT path FROM '${AC}' WHERE path = $1`, [`/task-assignments/${assigned.assignment_id}`]);
expect('members cannot read the record', recordAsMember.length === 0, JSON.stringify(recordAsMember));

console.log(failed ? `\n${failed} failed (repository ${repo})` : `\nall passed (repository ${repo})`);
process.exit(failed ? 1 : 0);
