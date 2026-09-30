/**
 * Expiry: a visitor session whose `expires_at` has passed is deleted with its
 * conversations; a live one, the agent's side of the conversation, and
 * anything outside /visitors are untouched.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from '../../../../../../../ai-tools/tests/support/fake-raisin.mjs';

const { purge_expired, isExpired } = await import('./index.js');

const AC = 'raisin:access_control';
const OLD = 'a'.repeat(32);
const LIVE = 'b'.repeat(32);
const NOW = '2026-09-30T12:00:00.000Z';

function world() {
  const f = fakeRaisin();
  const deleted = [];
  f.api.nodes.delete = async (ws, path) => {
    deleted.push(`${ws}:${path}`);
    for (const k of [...f.nodes.keys()]) {
      const [kws, kpath] = k.split('\u0000');
      if (kws === ws && (kpath === path || kpath.startsWith(`${path}/`))) f.nodes.delete(k);
    }
  };
  f.put(AC, `/visitors/${OLD}`, {
    node_type: 'raisin:VisitorSession',
    properties: { agent_path: '/agents/site', expires_at: '2026-09-30T11:59:00.000Z' },
  });
  f.put(AC, `/visitors/${OLD}/inbox/chats/vchat-1`, { node_type: 'raisin:Conversation' });
  f.put(AC, `/visitors/${LIVE}`, {
    node_type: 'raisin:VisitorSession',
    properties: { agent_path: '/agents/site', expires_at: '2026-10-01T12:00:00.000Z' },
  });
  f.put('ai', '/agents/site/inbox/chats/vchat-1', { node_type: 'raisin:Conversation' });
  f.put('ai', '/agents/site/inbox/chats/vchat-1/msg-1/cost-record', { node_type: 'raisin:AICostRecord' });
  return { f, deleted };
}

test('an expired session is purged with its conversations; a live one stays', async () => {
  const { f, deleted } = world();
  const res = await purge_expired({ now: NOW });
  assert.deepEqual(res, { purged: 1, failed: 0 });
  assert.deepEqual(deleted, [`${AC}:/visitors/${OLD}`]);
  assert.equal(f.nodes.get(`${AC}\u0000/visitors/${OLD}/inbox/chats/vchat-1`), undefined);
  assert.ok(f.nodes.get(`${AC}\u0000/visitors/${LIVE}`), 'the live session stays');
});

test('the agent side (cost records, tool audit) is kept', async () => {
  const { f } = world();
  await purge_expired({ now: NOW });
  assert.ok(f.nodes.get('ai\u0000/agents/site/inbox/chats/vchat-1/msg-1/cost-record'));
});

test('a failed delete does not stop the sweep', async () => {
  const { f } = world();
  f.put(AC, `/visitors/${'c'.repeat(32)}`, {
    node_type: 'raisin:VisitorSession',
    properties: { expires_at: '2026-09-01T00:00:00.000Z' },
  });
  const real = f.api.nodes.delete;
  f.api.nodes.delete = async (ws, path) => {
    if (path.endsWith(OLD)) throw new Error('boom');
    return real(ws, path);
  };
  const res = await purge_expired({ now: NOW });
  assert.deepEqual(res, { purged: 1, failed: 1 });
});

test('expiry reads expires_at, else a day after the last activity', () => {
  assert.equal(isExpired({ expires_at: '2026-09-30T11:00:00Z' }, NOW), true);
  assert.equal(isExpired({ expires_at: '2026-09-30T13:00:00Z' }, NOW), false);
  assert.equal(isExpired({ last_activity_at: '2026-09-30T00:00:00Z' }, NOW), false);
  assert.equal(isExpired({ last_activity_at: '2026-09-29T11:00:00Z' }, NOW), true);
  assert.equal(isExpired({}, NOW), true);
});
