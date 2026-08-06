// Run with: node --test .../adapters/google-calendar/index.test.mjs
//
// index.js is loaded the way the engine loads it — a bare script whose entry
// point is the global `handler` — so there is nothing to import. `raisin` is
// injected as a parameter of the wrapper, which is how the host provides it.
//
// Everything here is about ONE defect class: a cancelled INSTANCE of a
// recurring series. It is not a deleted event, it is the only evidence that an
// occurrence does not happen, and the expander suppresses a projected
// occurrence solely on the existence of an exception node at that slot
// (`calendar_expand/rebuild.rs`). Lose it in either phase and a cancelled
// meeting is regenerated forever, with no error anywhere.

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8');
const load = new Function('raisin', `${src}\nreturn handler;`);

const mount = { mount_id: 'm1', mount_path: '/cal', remote_root: 'primary', sync_config: {} };
const credential = { access_token: 'tok' };

/** A handler whose fetch answers `body`, recording every URL it was given. */
function withFetch(body) {
  const urls = [];
  const handler = load({
    http: {
      fetch(url) {
        urls.push(url);
        return { status: 200, headers: {}, body };
      },
    },
  });
  return { handler, urls };
}

const MASTER_ID = 'abc123';
const CANCELLED_INSTANCE = {
  id: `${MASTER_ID}_20260915T070000Z`,
  status: 'cancelled',
  recurringEventId: MASTER_ID,
  originalStartTime: { dateTime: '2026-09-15T09:00:00+02:00' },
  etag: '"1"',
};
const DELETED_SINGLE = { id: 'gone-1', status: 'cancelled', etag: '"2"' };

test('list asks for deleted records: a cancelled instance is invisible without it', () => {
  const { handler, urls } = withFetch({ items: [] });
  handler({ operation: 'list', params: {}, credential, mount });
  assert.match(
    urls[0],
    /showDeleted=true/,
    'the full walk deletes what it does not list, so an exception node imported ' +
      'by a delta is pruned by the next full pass and the ghost comes back'
  );
});

test('list materializes a cancelled instance and drops a deleted event', () => {
  const { handler } = withFetch({ items: [CANCELLED_INSTANCE, DELETED_SINGLE] });
  const out = handler({ operation: 'list', params: {}, credential, mount });

  assert.equal(out.items.length, 1, 'exactly one of the two is an item');
  const item = out.items[0];
  assert.equal(item.external_id, CANCELLED_INSTANCE.id);
  assert.equal(item.metadata.recurring_event_id, MASTER_ID);
  assert.deepEqual(item.metadata.original_start, CANCELLED_INSTANCE.originalStartTime);
  assert.equal(item.metadata.status, 'cancelled');
  // A genuinely deleted event must NOT come back as a node: the full reconcile
  // already deletes what it does not list, which is the right treatment.
  assert.equal(
    out.items.some((i) => i.external_id === DELETED_SINGLE.id),
    false
  );
});

test('get_changes reports a cancelled instance as an update, a deleted event as a delete', () => {
  const { handler } = withFetch({
    items: [CANCELLED_INSTANCE, DELETED_SINGLE],
    nextSyncToken: 'tok-2',
  });
  const out = handler({
    operation: 'get_changes',
    params: { since_token: 'tok-1' },
    credential,
    mount,
  });

  const byId = Object.fromEntries(
    out.items.map((i) => [i.item.external_id, i])
  );
  assert.equal(
    byId[CANCELLED_INSTANCE.id].type,
    'updated',
    'a cancelled occurrence has no node to delete — it needs one CREATED, because ' +
      'the exception node is the suppression record'
  );
  assert.equal(
    byId[CANCELLED_INSTANCE.id].item.metadata.recurring_event_id,
    MASTER_ID
  );
  assert.equal(byId[DELETED_SINGLE.id].type, 'deleted');
});
