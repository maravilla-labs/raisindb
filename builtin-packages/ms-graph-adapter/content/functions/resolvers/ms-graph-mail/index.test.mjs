// Run with: node --test builtin-packages/ms-graph-adapter/content/functions/resolvers/ms-graph-mail/
//
// index.js is loaded the way the engine loads it — a bare script whose entry
// point is the global `handler` — so there is nothing to import.

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8');
const handler = new Function(`${src}\nreturn handler;`)();

function resolve(field_diff, extra = {}) {
  return handler({
    operation: 'resolve_conflict',
    local: { properties: {} },
    remote: null,
    conflict: 'update: the message changed in Microsoft 365 since it was read (HTTP 412)',
    base_etag: 'W/"CQAAABYAAAA="',
    field_diff,
    mount: { mount_id: 'm1' },
    ...extra,
  });
}

test('marking a message unread is deliberate, so it survives the race', () => {
  assert.deepEqual(resolve({ unread: { local: true, pushed: false } }), {
    resolution: 'local_wins',
  });
});

test('marking a message read yields to the remote', () => {
  // Nothing is gained by forcing it: the remote is already at least as far
  // along on the only axis this mount pushes.
  assert.deepEqual(resolve({ unread: { local: false, pushed: true } }), {
    resolution: 'remote_wins',
  });
});

test('categories merge to the union — nobody loses a label', () => {
  const out = resolve({ labels: { local: ['Blue', 'Urgent'], pushed: ['Blue'] } });
  assert.equal(out.resolution, 'merged');
  assert.deepEqual(out.fields.labels, ['Blue', 'Urgent']);
});

test('the union takes both sides, deduplicated, local order first', () => {
  const out = resolve({ labels: { local: ['Urgent'], pushed: ['Blue', 'Urgent'] } });
  assert.deepEqual(out.fields.labels, ['Urgent', 'Blue']);
});

test('an unhandled field parks rather than guessing', () => {
  // The whole point: a resolver that fell through to a default would reach
  // someone's mailbox with a decision nobody made.
  const out = resolve({ folder: { local: 'archive', pushed: 'inbox' } });
  assert.equal(out.resolution, 'park');
  assert.match(out.reason, /no rule for this conflict/);
});

test('a diff with no local change at all parks', () => {
  assert.equal(resolve({ unread: { local: null, pushed: false } }).resolution, 'park');
  assert.equal(resolve({}).resolution, 'park');
});

test('an operation this function does not handle parks with a reason', () => {
  // Silence here would look like a resolver that keeps declining; the reason
  // reaches the mount's writeback_last_error.
  const out = handler({ operation: 'to_node' });
  assert.equal(out.resolution, 'park');
  assert.match(out.reason, /resolve_conflict/);
});
