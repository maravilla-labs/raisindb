// Run with: node --test .../google-calendar-default/index.test.mjs
//
// index.js is loaded the way the engine loads it — a bare script whose entry
// point is the global `handler` — so there is nothing to import.

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8');
const handler = new Function(`${src}\nreturn handler;`)();

const mount = { mount_id: 'm1', mount_path: '/cal', remote_root: 'primary', sync_config: {} };

function map(metadata, item = {}) {
  return handler({
    operation: 'to_node',
    mount,
    external_item: {
      external_id: 'evt-1',
      name: 'evt-1',
      web_url: 'https://calendar/evt-1',
      metadata: { calendar_id: 'primary', ...metadata },
      ...item,
    },
  });
}

test('reports no writeback: the adapter has no write case at all', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount }), {
    to_external: false,
  });
  assert.equal(handler({ operation: 'to_external', node: {}, mount }), null);
});

test('an absent operation still means to_node', () => {
  const out = handler({
    mount,
    external_item: { external_id: 'e', name: 'e', metadata: { summary: 'Hi' } },
  });
  assert.equal(out.node_type, 'raisin:Event');
  assert.equal(out.properties.title, 'Hi');
});

test('an offset-bearing dateTime yields a fixed-width UTC instant plus local time', () => {
  const p = map({
    summary: 'Standup',
    start: '2026-08-05T09:00:00+02:00',
    end: '2026-08-05T09:15:00+02:00',
    start_timezone: 'Europe/Zurich',
  }).properties;
  assert.equal(p.start_utc, '2026-08-05T07:00:00Z');
  assert.equal(p.end_utc, '2026-08-05T07:15:00Z');
  assert.match(p.start_utc, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
  // Local is the wall clock the user agreed to, with the offset stripped.
  assert.equal(p.start_local, '2026-08-05T09:00:00');
  assert.equal(p.timezone, 'Europe/Zurich');
});

test('an all-day event is a bare local date and a midnight UTC instant', () => {
  const p = map({
    summary: 'Holiday',
    all_day: true,
    start: '2026-08-05',
    end: '2026-08-06',
  }).properties;
  assert.equal(p.start_local, '2026-08-05');
  assert.equal(p.start_utc, '2026-08-05T00:00:00Z');
  // Exclusive end, matching RFC 5545 DTEND and both providers.
  assert.equal(p.end_utc, '2026-08-06T00:00:00Z');
});

test('raw Google attendees become the same objects the Graph mapper emits', () => {
  const p = map({
    summary: 'Review',
    attendees: [
      { email: 'ada@example.com', displayName: 'Ada Lovelace', responseStatus: 'accepted' },
      { email: 'room@example.com', resource: true, responseStatus: 'needsAction' },
      { email: 'bob@example.com', optional: true, responseStatus: 'declined' },
    ],
  }).properties;
  assert.deepEqual(p.attendees, [
    { email: 'ada@example.com', name: 'Ada Lovelace', type: 'required', response: 'accepted' },
    { email: 'room@example.com', name: null, type: 'resource', response: 'needs_action' },
    { email: 'bob@example.com', name: null, type: 'optional', response: 'declined' },
  ]);
});

test('an absent attendee list is null on this side too, never []', () => {
  assert.equal(map({ summary: 'X' }).properties.attendees, null);
  assert.equal(map({ summary: 'X', attendees: [] }).properties.attendees, null);
});

test('organizer is split into a bare email and a name', () => {
  let p = map({ summary: 'X', organizer: 'ada@example.com' }).properties;
  assert.equal(p.organizer_email, 'ada@example.com');
  assert.equal(p.organizer_name, null);

  p = map({
    summary: 'X',
    organizer: { email: 'ada@example.com', displayName: 'Ada Lovelace' },
  }).properties;
  assert.equal(p.organizer_email, 'ada@example.com');
  assert.equal(p.organizer_name, 'Ada Lovelace');
});

test('my_response comes from the attendee row marked self', () => {
  assert.equal(
    map({
      summary: 'X',
      attendees: [
        { email: 'ada@example.com', responseStatus: 'accepted' },
        { email: 'me@example.com', self: true, responseStatus: 'needsAction' },
      ],
    }).properties.my_response,
    'needs_action',
  );
  assert.equal(
    map({ summary: 'X', organizer: { email: 'me@example.com', self: true } }).properties
      .my_response,
    'organizer',
  );
  assert.equal(map({ summary: 'X' }).properties.my_response, null);
});

test('status is the RFC 5545 event status; transparency is show_as', () => {
  let p = map({ summary: 'X', status: 'tentative', transparency: 'transparent' }).properties;
  assert.equal(p.status, 'tentative');
  assert.equal(p.show_as, 'free');
  assert.equal(p.my_response, null);

  p = map({ summary: 'X', transparency: 'opaque' }).properties;
  assert.equal(p.status, 'confirmed');
  assert.equal(p.show_as, 'busy');
});

test('a cancelled event is materialized, not skipped — the Graph mapper agrees', () => {
  // This mapper used to return null here while Graph kept the node, so the same
  // meeting existed on one provider and vanished on the other.
  const out = map({ summary: 'Dropped', status: 'cancelled' });
  assert.notEqual(out, null);
  assert.equal(out.properties.status, 'cancelled');
});

test("Google's RRULE array is the column's shape verbatim", () => {
  const p = map({
    summary: 'Standup',
    recurrence: ['RRULE:FREQ=WEEKLY;BYDAY=TU', 'EXDATE;TZID=Europe/Zurich:20260811T090000'],
  }).properties;
  assert.deepEqual(p.recurrence, [
    'RRULE:FREQ=WEEKLY;BYDAY=TU',
    'EXDATE;TZID=Europe/Zurich:20260811T090000',
  ]);
  assert.equal(p.recurrence_type, 'series_master');
});

test('recurring_event_id separates an occurrence from an exception', () => {
  assert.equal(map({ summary: 'X' }).properties.recurrence_type, 'single');

  const occ = map({ summary: 'X', recurring_event_id: 'master-1' }).properties;
  assert.equal(occ.recurrence_type, 'occurrence');
  assert.equal(occ.series_master_external_id, 'master-1');

  const exc = map({
    summary: 'X',
    recurring_event_id: 'master-1',
    original_start: { dateTime: '2026-08-11T09:00:00+02:00' },
  }).properties;
  assert.equal(exc.recurrence_type, 'exception');
  assert.equal(exc.original_start_utc, '2026-08-11T07:00:00Z');
  assert.equal(exc.original_start_local, '2026-08-11T09:00:00');
});

test('Google has no coordinates, and says so rather than leaving it implicit', () => {
  assert.equal(map({ summary: 'X', location: 'Room 3' }).properties.location_geo, null);
  assert.equal(map({ summary: 'X', location: 'Room 3' }).properties.location, 'Room 3');
});

test('a description is written only when the adapter returned one', () => {
  assert.equal('description_html' in map({ summary: 'X' }).properties, false);
  assert.equal(map({ summary: 'X', description: '<p>hi</p>' }).properties.description_html, '<p>hi</p>');
});

// ---------------------------------------------------------------------------
// Cross-provider agreement.
//
// The point of raisin:Event v2 is that a consumer never branches on provider,
// and the only way that stays true is to compare the two mappers directly. This
// reaches across package boundaries on purpose: a divergence reintroduced in
// either file has to fail somewhere, and there is no shared file to put it in.
const graphSrc = readFileSync(
  new URL('../../../../../ms-graph-adapter/content/functions/mappers/ms-graph-calendar/index.js', import.meta.url),
  'utf8',
);
const graphHandler = new Function(`${graphSrc}\nreturn handler;`)();

function graphMap(metadata) {
  return graphHandler({
    operation: 'to_node',
    mount: { mount_id: 'm1', mount_path: '/cal', remote_root: null, sync_config: {} },
    external_item: { external_id: 'e', name: 'e', web_url: 'https://outlook/e', metadata },
  });
}

test('both mappers emit the same raisin:Event column set', () => {
  const google = Object.keys(map({ summary: 'Standup' }).properties).sort();
  const graph = Object.keys(graphMap({ subject: 'Standup' }).properties).sort();
  assert.deepEqual(graph, google, 'a column present on one provider only is a silent divergence');
});

test('both mappers describe the same meeting identically', () => {
  const google = map({
    summary: 'Standup',
    start: '2026-08-05T09:00:00+02:00',
    end: '2026-08-05T09:15:00+02:00',
    start_timezone: 'Europe/Berlin',
    location: 'Room 3',
    status: 'cancelled',
    organizer: { email: 'ada@example.com', displayName: 'Ada Lovelace' },
    attendees: [{ email: 'bob@example.com', displayName: 'Bob', optional: true, responseStatus: 'tentative' }],
    calendar_id: 'calendar',
  }).properties;

  const graph = graphMap({
    subject: 'Standup',
    start: '2026-08-05T07:00:00.0000000Z',
    end: '2026-08-05T07:15:00.0000000Z',
    start_tz: 'W. Europe Standard Time',
    location: 'Room 3',
    status: 'cancelled',
    organizer: 'Ada Lovelace <ada@example.com>',
    attendees: [
      {
        type: 'optional',
        status: { response: 'tentativelyAccepted' },
        emailAddress: { name: 'Bob', address: 'bob@example.com' },
      },
    ],
  }).properties;

  for (const key of [
    'title',
    'calendar_id',
    'start_utc',
    'end_utc',
    'timezone',
    'all_day',
    'recurrence_type',
    'status',
    'organizer_email',
    'organizer_name',
    'attendees',
    'location',
  ]) {
    assert.deepEqual(graph[key], google[key], `providers disagree on '${key}'`);
  }
  assert.equal(graph.status, 'cancelled', 'neither mapper may drop a cancelled event');
});
