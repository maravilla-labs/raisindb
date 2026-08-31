// Run with: node --test .../google-calendar-outbox/index.test.mjs
//
// index.js is loaded the way the engine loads a MAPPER — a bare script whose
// entry point is the global `handler` — so there is nothing to import. (The
// ADAPTER next door is an ES module and is imported; the two loaders differ
// because the two files genuinely differ.)

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8');
const handler = new Function(`${src}\nreturn handler;`)();

const mount = {
  mount_id: 'm1',
  mount_path: '/calendars/team/rsvp',
  remote_root: 'primary',
  sync_config: {},
};

const rsvpNode = (properties) => ({ node_type: 'raisin:CalendarAction', properties });

test('an outbox declares the write direction and imports nothing', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount }), {
    to_external: true,
  });
  // Null, not absent: a submit mount that also tried to IMPORT would
  // materialize answered invitations as new commands.
  assert.equal(handler({ operation: 'to_node', external_item: { external_id: 'e' }, mount }), null);
  assert.equal(handler({ mount, external_item: { external_id: 'e' } }), null);
});

test('an RSVP becomes an intent-shaped command addressed by the PROVIDER id', () => {
  const out = handler({
    operation: 'to_external',
    mount,
    node: rsvpNode({ action: 'decline', target_external_id: 'evt-1', comment: 'clash' }),
  });
  assert.deepEqual(out, {
    payload: { action: 'decline', body: { comment: 'clash', send_response: true } },
    external_id: 'evt-1',
  });
});

test('the payload carries NO attendees array — that is the adapter half', () => {
  // A mapper is I/O-free by contract, so it cannot know which attendee row is
  // the caller's (`self === true` is only visible on the event itself). The
  // array it COULD build — just the caller's row — is the one that deletes
  // every other guest, because events.patch overwrites array fields wholesale.
  const out = handler({
    operation: 'to_external',
    mount,
    node: rsvpNode({ action: 'accept', target_external_id: 'evt-1' }),
  });
  assert.equal('attendees' in out.payload.body, false);
  assert.deepEqual(Object.keys(out.payload.body), ['send_response']);
});

test('notifying the organizer is stated, not inherited, and defaults to true', () => {
  const on = handler({
    operation: 'to_external',
    mount,
    node: rsvpNode({ action: 'accept', target_external_id: 'evt-1' }),
  });
  assert.equal(on.payload.body.send_response, true);

  const off = handler({
    operation: 'to_external',
    mount,
    node: rsvpNode({ action: 'accept', target_external_id: 'evt-1', send_response: false }),
  });
  assert.equal(off.payload.body.send_response, false);
});

test('an unsendable command is null, never a guess', () => {
  const cases = [
    ['no action', rsvpNode({ target_external_id: 'evt-1' })],
    ['unknown action', rsvpNode({ action: 'maybe', target_external_id: 'evt-1' })],
    ['no target', rsvpNode({ action: 'accept' })],
    // This package ships no mail adapter, so a mail command here would be a
    // command surface with nothing behind it — the exact shape that makes a
    // mount resolve as capable and then throw at drain time.
    ['a mail command', { node_type: 'raisin:OutboundMail', properties: { action: 'send' } }],
    ['no node', null],
  ];
  for (const [label, node] of cases) {
    assert.equal(handler({ operation: 'to_external', mount, node }), null, label);
  }
});

// ---------------------------------------------------------------------------
// Cross-provider agreement, the COMMAND direction.
//
// The read direction already has this (google-calendar-default's suite compares
// the two calendar mappers column for column). A raisin:CalendarAction is just
// as provider-neutral: `raisin:CalendarAction` and DEFAULT_COMMAND_NODE_TYPES
// are engine-side and identical for every provider, so the SAME node must be
// sendable through either outbox with the same intent. Reaching across the
// package boundary is deliberate — a divergence reintroduced in either file has
// to fail somewhere, and there is no shared file to put it in.
//
// NOTE: this readFileSync means moving or deleting either package breaks the
// other's suite. Fragile, and still the only cross-provider guard there is.
const graphSrc = readFileSync(
  new URL('../../../../../ms-graph-adapter/content/functions/mappers/ms-graph-outbox/index.js', import.meta.url),
  'utf8',
);
const graphHandler = new Function(`${graphSrc}\nreturn handler;`)();

test('both outboxes accept the same raisin:CalendarAction and agree on the intent', () => {
  const node = rsvpNode({
    action: 'tentative',
    target_external_id: 'evt-1',
    comment: 'might make it',
    send_response: false,
  });
  const google = handler({ operation: 'to_external', mount, node });
  const graph = graphHandler({ operation: 'to_external', mount, node });

  assert.equal(graph.payload.action, google.payload.action, 'the action is the node\'s own word');
  assert.equal(graph.external_id, google.external_id, 'both address the PROVIDER id');
  assert.equal(graph.payload.body.comment, google.payload.body.comment);
  // Same decision, different provider spelling: Graph's /accept action takes
  // `sendResponse`, Google's events.patch takes a `sendUpdates` query parameter
  // the adapter derives from `send_response`. Only the spelling may differ.
  assert.equal(graph.payload.body.sendResponse, false);
  assert.equal(google.payload.body.send_response, false);

  // And both declare the same direction.
  assert.deepEqual(
    graphHandler({ operation: 'mapper_capabilities', mount }),
    handler({ operation: 'mapper_capabilities', mount }),
  );
});
