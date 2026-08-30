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

test('a naive datetime in a NAMED zone yields local time and a NULL UTC instant', () => {
  // Graph returns wall-clock with no offset. For a named zone the instant needs
  // a tz database, and `Intl` does not exist in QuickJS, so start_utc must stay
  // null rather than lie. The zone itself is still recorded, so the wall clock
  // remains readable and a client with a real tz database can convert it.
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

// The production defect, exactly as it read in RaisinDB: start_local
// 2026-08-31T21:30:00, timezone UTC, and start_utc NULL — so Studio drew the
// wall clock verbatim at 21:30 while Outlook, in Europe/Zurich, showed 23:30.
// The instant was derivable the whole time: Graph sends the zone in a SEPARATE
// field from the wall clock, and the mapper only ever looked at the string.
test('a naive datetime whose zone is UTC is an exact instant, not a null', () => {
  const p = map({
    subject: 'Evening call',
    start: '2026-08-31T21:30:00.0000000',
    start_tz: 'UTC',
    end: '2026-09-01T00:00:00.0000000',
    end_tz: 'UTC',
  }).properties;
  assert.equal(p.start_utc, '2026-08-31T21:30:00Z');
  // The event ends at midnight UTC — the next DAY. A client converting the
  // instant into Europe/Zurich draws 23:30-02:00 across two days; formatting
  // start_local alone drew it inside 31 August and lost the crossing.
  assert.equal(p.end_utc, '2026-09-01T00:00:00Z');

  // Nothing else moved: an already-synced event gains the pair and changes in
  // no other way, so it must not re-write as "modified" beyond these columns.
  assert.equal(p.start_local, '2026-08-31T21:30:00');
  assert.equal(p.end_local, '2026-09-01T00:00:00');
  assert.equal(p.timezone, 'UTC');
  assert.equal(p.all_day, false);

  // Graph's seven fractional digits are dropped, not rounded: the column is
  // second-resolution and every other row is fixed-width.
  assert.match(p.start_utc, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
});

test('the UTC alias zones convert; a zone that is only SOMETIMES UTC does not', () => {
  for (const tz of ['UTC', 'Etc/UTC', 'Etc/GMT', 'GMT']) {
    assert.equal(
      map({ subject: 'X', start: '2026-08-31T21:30:00.0000000', start_tz: tz }).properties
        .start_utc,
      '2026-08-31T21:30:00Z',
      tz + ' is UTC on every date'
    );
  }
  // Windows "GMT Standard Time" is Europe/London: UTC in winter, UTC+1 in
  // summer. Treating it as UTC would move every British summer meeting by an
  // hour, so it stays null even though the name looks like GMT.
  const london = map({
    subject: 'X',
    start: '2026-08-31T21:30:00.0000000',
    start_tz: 'GMT Standard Time',
  }).properties;
  assert.equal(london.start_utc, null);
  assert.equal(london.timezone, 'Europe/London');
});

test('end_tz falls back to start_tz, and a zoneless naive value is still null', () => {
  // Some Graph projections omit the end's timeZone; both ends of one event are
  // always in one zone, so the start's answers for it.
  const p = map({
    subject: 'X',
    start: '2026-08-31T21:30:00.0000000',
    start_tz: 'UTC',
    end: '2026-08-31T22:00:00.0000000',
  }).properties;
  assert.equal(p.end_utc, '2026-08-31T22:00:00Z');

  // No zone at all is not an invitation to assume one.
  const bare = map({ subject: 'X', start: '2026-08-31T21:30:00.0000000' }).properties;
  assert.equal(bare.start_utc, null);
  assert.equal(bare.timezone, null);
});

// to_external already preferred (timezone + start_local) over start_utc, so an
// event that has just GAINED a utc pair must push exactly the payload it pushed
// before — otherwise this fix would reschedule every UTC event at the provider.
test('gaining a utc pair does not change what is pushed to Graph', () => {
  const properties = {
    start_local: '2026-08-31T21:30:00',
    end_local: '2026-09-01T00:00:00',
    timezone: 'UTC',
    all_day: false,
  };
  const before = handler({
    operation: 'to_external',
    node: { properties },
    mount,
    fields: ['start_local'],
    intent: 'update',
  });
  const after = handler({
    operation: 'to_external',
    node: {
      properties: {
        ...properties,
        start_utc: '2026-08-31T21:30:00Z',
        end_utc: '2026-09-01T00:00:00Z',
      },
    },
    mount,
    fields: ['start_local'],
    intent: 'update',
  });
  assert.deepEqual(after, before);
  assert.deepEqual(after.payload.start, {
    dateTime: '2026-08-31T21:30:00',
    timeZone: 'UTC',
  });
});

// UTC_ZONES and WINDOWS_TO_IANA are BARE objects, so a `!obj[name]` test reaches
// Object.prototype: "constructor" and "toString" are members of every one of
// them. Membership in UTC_ZONES is a claim the offset is zero forever, and a
// name we have proved nothing about must not stamp a Z — nor put a FUNCTION in
// the `timezone` string column, which is what `WINDOWS_TO_IANA[name] || null`
// returned for the same names.
test('a zone name inherited from Object.prototype is not a zone', () => {
  for (const tz of ['constructor', 'toString', 'valueOf', 'hasOwnProperty']) {
    const p = map({ subject: 'X', start: '2026-08-31T21:30:00.0000000', start_tz: tz })
      .properties;
    assert.equal(p.start_utc, null, tz + ' proves nothing about the offset');
    assert.equal(p.timezone, null, tz + ' is not an IANA zone');
  }
});
