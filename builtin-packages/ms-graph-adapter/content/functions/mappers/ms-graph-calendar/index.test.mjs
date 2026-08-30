// Run with: node --test builtin-packages/ms-graph-adapter/content/functions/mappers/ms-graph-calendar/
//
// The mapper is a MODULE: index.js imports ./time.js and ./recurrence.js, and
// the engine resolves those siblings from the function node's own files. So it
// is imported here, exactly as the adapter's own suite imports its modules.
// `new Function(src)` — what this file used to do — cannot evaluate an import
// statement at all. It is NOT the only reader of this file: the google-calendar
// mapper's suite loads this same index.js for its cross-provider convergence
// tests, so it has to import it too or those two tests die as an async
// SyntaxError that node reports after the run.

import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

const mount = { mount_id: 'm1', mount_path: '/cal', remote_root: null, sync_config: {} };

function map(metadata, item = {}) {
  return handler({
    operation: 'to_node',
    mount,
    external_item: {
      external_id: 'evt-1',
      name: 'evt-1',
      web_url: 'https://outlook/evt-1',
      metadata,
      ...item,
    },
  });
}

test('declares two-way writeback, and an empty node maps to nothing', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount }), {
    to_external: true,
  });
  // An empty node has no emittable fields; the mapper declines rather than
  // sending an empty PATCH.
  assert.equal(handler({ operation: 'to_external', node: {}, mount }), null);
});

// The invite-spam guard, from the mapper's side: `attendees` must be emitted
// ONLY when it is in the caller's field list. (The engine's half of the same
// guard passes only the fields that actually DIVERGED — Graph resends meeting
// invitations to every attendee whenever `attendees` appears in a PATCH,
// changed or not.)
test('attendees is emitted only when the field list names it', () => {
  const node = {
    properties: {
      title: 'Renamed',
      attendees: [{ email: 'a@example.com', name: 'A' }],
    },
  };
  const titleOnly = handler({
    operation: 'to_external',
    node,
    mount,
    fields: ['title'],
    intent: 'update',
  });
  assert.ok(titleOnly && titleOnly.payload);
  assert.equal(titleOnly.payload.subject, 'Renamed');
  assert.equal(
    'attendees' in titleOnly.payload,
    false,
    'a title-only update must not carry the attendee list'
  );

  const withAttendees = handler({
    operation: 'to_external',
    node,
    mount,
    fields: ['title', 'attendees'],
    intent: 'update',
  });
  assert.ok(withAttendees && withAttendees.payload.attendees);
  assert.equal(withAttendees.payload.attendees.length, 1);
});

test('an absent operation still means to_node', () => {
  const out = handler({
    mount,
    external_item: { external_id: 'e', name: 'e', metadata: { subject: 'Hi' } },
  });
  assert.equal(out.node_type, 'raisin:Event');
  assert.equal(out.properties.title, 'Hi');
});

test('a naive Graph datetime yields local time and a NULL UTC instant', () => {
  // Graph returns wall-clock with no offset; converting needs a tz database the
  // mapper does not have, so start_utc must stay null rather than lie.
  const p = map({
    subject: 'Standup',
    start: '2026-08-05T09:00:00.0000000',
    start_tz: 'W. Europe Standard Time',
    end: '2026-08-05T09:15:00.0000000',
    end_tz: 'W. Europe Standard Time',
  }).properties;
  assert.equal(p.start_utc, null);
  assert.equal(p.end_utc, null);
  assert.equal(p.start_local, '2026-08-05T09:00:00');
  assert.equal(p.end_local, '2026-08-05T09:15:00');
  assert.equal(p.timezone, 'Europe/Berlin');
});

test('a zoned datetime (Prefer: outlook.timezone="UTC") becomes a fixed-width instant', () => {
  const p = map({ subject: 'X', start: '2026-08-05T07:00:00.0000000Z' }).properties;
  assert.equal(p.start_utc, '2026-08-05T07:00:00Z');
  assert.match(p.start_utc, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
});

test('an all-day event is a bare local date and a midnight UTC instant', () => {
  const p = map({
    subject: 'Holiday',
    all_day: true,
    start: '2026-08-05T00:00:00.0000000',
    end: '2026-08-06T00:00:00.0000000',
  }).properties;
  assert.equal(p.all_day, true);
  assert.equal(p.start_local, '2026-08-05');
  assert.equal(p.start_utc, '2026-08-05T00:00:00Z');
  assert.equal(p.end_utc, '2026-08-06T00:00:00Z');
});

test('an unmappable Windows zone is null, never a guess', () => {
  assert.equal(map({ subject: 'X', start_tz: 'Nowhere Standard Time' }).properties.timezone, null);
  assert.equal(map({ subject: 'X', start_tz: 'Europe/Zurich' }).properties.timezone, 'Europe/Zurich');
});

test('"Name <addr>" attendees become objects; an empty list is null, not []', () => {
  const p = map({
    subject: 'Review',
    attendees: ['Ada Lovelace <ada@example.com>', 'bob@example.com'],
    organizer: 'Ada Lovelace <ada@example.com>',
  }).properties;
  assert.deepEqual(p.attendees, [
    { email: 'ada@example.com', name: 'Ada Lovelace', type: 'required', response: null },
    { email: 'bob@example.com', name: null, type: 'required', response: null },
  ]);
  assert.equal(p.organizer_email, 'ada@example.com');
  assert.equal(p.organizer_name, 'Ada Lovelace');
  assert.equal(map({ subject: 'X' }).properties.attendees, null);
  assert.equal(map({ subject: 'X', attendees: [] }).properties.attendees, null);
});

test('raw Graph attendee objects carry type and RSVP through', () => {
  const p = map({
    subject: 'Review',
    attendees: [
      {
        type: 'optional',
        status: { response: 'tentativelyAccepted' },
        emailAddress: { name: 'Bob', address: 'bob@example.com' },
      },
    ],
  }).properties;
  assert.deepEqual(p.attendees, [
    { email: 'bob@example.com', name: 'Bob', type: 'optional', response: 'tentative' },
  ]);
});

test("Graph's one conflated status string is split into three columns", () => {
  // Free/busy vocabulary (what the projected list path returns).
  let p = map({ subject: 'X', status: 'oof' }).properties;
  assert.equal(p.status, 'confirmed');
  assert.equal(p.show_as, 'out_of_office');
  assert.equal(p.my_response, null);

  // RSVP vocabulary (what the unprojected delta/get paths return for the SAME
  // event) — the path-dependence that made this column unreadable.
  p = map({ subject: 'X', status: 'tentativelyAccepted' }).properties;
  assert.equal(p.status, 'confirmed');
  assert.equal(p.show_as, null);
  assert.equal(p.my_response, 'tentative');
});

test('a cancelled event is materialized, not skipped', () => {
  const out = map({ subject: 'Dropped', status: 'cancelled' });
  assert.notEqual(out, null);
  assert.equal(out.properties.status, 'cancelled');
});

test('patternedRecurrence becomes RFC 5545 lines', () => {
  const weekly = map({
    subject: 'Standup',
    recurrence: JSON.stringify({
      pattern: { type: 'weekly', interval: 1, daysOfWeek: ['tuesday', 'thursday'] },
      range: { type: 'endDate', startDate: '2026-08-04', endDate: '2026-12-31' },
    }),
  }).properties;
  assert.deepEqual(weekly.recurrence, [
    'RRULE:FREQ=WEEKLY;BYDAY=TU,TH;UNTIL=20261231T235959Z',
  ]);
  assert.equal(weekly.recurrence_type, 'series_master');

  const monthly = map({
    subject: 'Retro',
    recurrence: {
      pattern: { type: 'relativeMonthly', interval: 2, daysOfWeek: ['friday'], index: 'last' },
      range: { type: 'numbered', numberOfOccurrences: 6 },
    },
  }).properties;
  assert.deepEqual(monthly.recurrence, [
    'RRULE:FREQ=MONTHLY;INTERVAL=2;BYDAY=FR;BYSETPOS=-1;COUNT=6',
  ]);
});

test('a non-recurring event is single with a null recurrence', () => {
  const p = map({ subject: 'One-off' }).properties;
  assert.equal(p.recurrence_type, 'single');
  assert.equal(p.recurrence, null);
  assert.equal(p.series_master_external_id, null);
});

test('unparseable recurrence degrades to null rather than throwing', () => {
  assert.equal(map({ subject: 'X', recurrence: '{not json' }).properties.recurrence, null);
  assert.equal(map({ subject: 'X', recurrence: '{}' }).properties.recurrence, null);
});

test('calendar_id falls back to the adapter default instead of null', () => {
  // Reading mount.remote_root alone left this null on every default-calendar
  // mount, so a per-calendar filter silently covered only Google.
  assert.equal(map({ subject: 'X' }).properties.calendar_id, 'calendar');
  const named = handler({
    operation: 'to_node',
    mount: { ...mount, remote_root: 'AAMkAD' },
    external_item: { external_id: 'e', name: 'e', metadata: { subject: 'X' } },
  });
  assert.equal(named.properties.calendar_id, 'AAMkAD');
});

test('a body is written only when the adapter returned one', () => {
  assert.equal('description_html' in map({ subject: 'X' }).properties, false);
  assert.equal('description_text' in map({ subject: 'X' }).properties, false);
  const p = map({ subject: 'X', body: '<p>hi</p>', body_type: 'html' }).properties;
  assert.equal(p.description_html, '<p>hi</p>');
});
