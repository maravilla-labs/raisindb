// Run with: node --test .../adapters/google-calendar/index.test.mjs
//
// index.js is loaded the way the ENGINE loads it: an ES module whose EXPORTED
// `handler` is the entry point, importing its siblings the way the engine's
// module loader resolves them, with `raisin` reached as a HOST GLOBAL rather
// than as a wrapper argument (which is what the QuickJS runtime injects).
// Same shape as google-drive's suite; `tests_google_calendar_*.rs` hands
// QuickJS the same file set.
//
// THIS FILE DID NOT RUN FOR MONTHS. The loader used to be
// `readFileSync('./index.js')` + `new Function('raisin', src)`, which was
// correct while index.js was one flat script. The module split (0688af45)
// turned it into an ES module with five `import` statements, and `new Function`
// throws on the first one — so every test below died at load and node --test
// reported a single file-level failure that nobody read as "the cancelled
// instance guard is gone". Load it as a module; the assertions are untouched.
//
// Everything here is about ONE defect class: a cancelled INSTANCE of a
// recurring series. It is not a deleted event, it is the only evidence that an
// occurrence does not happen, and the expander suppresses a projected
// occurrence solely on the existence of an exception node at that slot
// (`calendar_expand/rebuild.rs`). Lose it in either phase and a cancelled
// meeting is regenerated forever, with no error anywhere.
//
// Plus the syncToken PARAMETER IDENTITY guard at the bottom, which nothing has
// ever had.

import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

const mount = { mount_id: 'm1', mount_path: '/cal', remote_root: 'primary', sync_config: {} };
const credential = { access_token: 'tok' };

/** A handler whose fetch answers `body`, recording every URL it was given. */
function withFetch(body) {
  const urls = [];
  globalThis.raisin = {
    http: {
      fetch(url) {
        urls.push(url);
        return { status: 200, headers: {}, body };
      },
    },
  };
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

// ---------------------------------------------------------------------------
// syncToken PARAMETER IDENTITY.
//
// Google's sync guide: "Each list request should use the same set of query
// parameters, including the initial request." The events.list reference names
// the ONLY exemptions — iCalUID, orderBy, privateExtendedProperty, q,
// sharedExtendedProperty, timeMin, timeMax, updatedMin — and adds that a
// syncToken request may not set showDeleted to false.
//
// The baseline used to omit showDeleted while the delta sent it. Best case the
// delta quietly dropped every cancellation (the exact records the three tests
// above exist to protect); worst case Google 400s, which this adapter reports
// as cursor_invalid, so the engine drops the token and the next run mints
// another mismatched baseline — a mount that full-reconciles forever and never
// has a working incremental feed. Nothing guarded it.

/** The query parameters of `url` as a Map, path discarded. */
function queryOf(url) {
  return new Map([...new URL(url).searchParams.entries()]);
}

// Exempt from the identity rule per the events.list reference; a syncToken
// request MUST drop these, so they are not part of the comparison.
const WINDOW_PARAMS = new Set([
  'iCalUID',
  'orderBy',
  'privateExtendedProperty',
  'q',
  'sharedExtendedProperty',
  'timeMin',
  'timeMax',
  'updatedMin',
  // Paging, not identity: either leg may or may not be mid-enumeration.
  'pageToken',
  'syncToken',
]);

test('the syncToken baseline asks for deleted records, exactly as the delta does', () => {
  const { handler, urls } = withFetch({ items: [], nextSyncToken: 'tok-1' });
  const out = handler({ operation: 'get_changes', params: {}, credential, mount });

  assert.equal(out.next_token, 'tok-1');
  assert.equal(
    queryOf(urls[0]).get('showDeleted'),
    'true',
    'a token minted without showDeleted cannot be spent on a request that has it'
  );
});

test('baseline and delta agree on every parameter that is part of the sync identity', () => {
  const baseline = withFetch({ items: [], nextSyncToken: 'tok-1' });
  baseline.handler({ operation: 'get_changes', params: {}, credential, mount });

  const delta = withFetch({ items: [], nextSyncToken: 'tok-2' });
  delta.handler({
    operation: 'get_changes',
    params: { since_token: 'tok-1' },
    credential,
    mount,
  });

  const identity = (url) =>
    [...queryOf(url)].filter(([k]) => !WINDOW_PARAMS.has(k)).sort();

  assert.deepEqual(
    identity(delta.urls[0]),
    identity(baseline.urls[0]),
    'the delta request must differ from the baseline ONLY in the parameters ' +
      'Google exempts (timeMin/timeMax/orderBy/q/...); any other difference is a ' +
      'different sync identity, i.e. dropped deletions or a permanent 400 loop'
  );
});

// ---------------------------------------------------------------------------
// submit: the RSVP command.
//
// These belong in the Rust QuickJS suite too — that is the only one CI runs —
// but they are here because this is the file that describes the shape.

/**
 * A handler whose fetch answers from a QUEUE, recording every request.
 *
 * An exhausted queue throws, so a test expecting N calls proves the adapter
 * made no more than N — which is the whole point for a command path.
 */
function stub(responses) {
  const calls = [];
  const queue = [...responses];
  globalThis.raisin = {
    http: {
      fetch(url, request) {
        calls.push({ url, request });
        const next = queue.shift();
        if (!next) throw new Error(`unexpected extra request: ${url}`);
        return { status: 200, headers: {}, body: {}, ...next };
      },
    },
  };
  return { handler, calls };
}

const ok = (body) => ({ status: 200, headers: {}, body });

const INVITE = {
  id: 'evt-1',
  etag: '"7"',
  attendees: [
    { email: 'ada@example.com', displayName: 'Ada', responseStatus: 'accepted' },
    { email: 'me@example.com', self: true, responseStatus: 'needsAction' },
    { email: 'room@example.com', resource: true, responseStatus: 'accepted' },
  ],
};

const rsvp = (action, body) => ({
  operation: 'submit',
  params: { payload: { action, body }, external_id: 'evt-1', idempotency_key: 'k' },
  credential,
  mount,
});

test('an RSVP PATCHes the WHOLE attendee list, with only the self row changed', () => {
  // events.patch: "Array fields, if specified, overwrite the existing arrays;
  // this discards any previous array elements." PATCHing only the self row
  // therefore DELETES EVERY OTHER GUEST from the meeting — and mails them all
  // about it. This is the assertion that stops that from ever being "simplified".
  const { handler, calls } = stub([ok(INVITE), ok({ id: 'evt-1', etag: '"8"' })]);
  const out = handler(rsvp('decline'));

  assert.equal(calls.length, 2, 'read then write, no more');
  assert.equal(calls[0].request.method, 'GET');
  assert.equal(calls[1].request.method, 'PATCH');

  const sent = JSON.parse(calls[1].request.body);
  assert.deepEqual(
    sent.attendees.map((a) => a.email),
    ['ada@example.com', 'me@example.com', 'room@example.com'],
    'every guest survives the RSVP, in order'
  );
  assert.deepEqual(sent.attendees[0], INVITE.attendees[0], 'other rows travel untouched');
  assert.deepEqual(sent.attendees[2], INVITE.attendees[2]);
  assert.equal(sent.attendees[1].responseStatus, 'declined');
  assert.equal(sent.attendees[1].self, true, 'the row keeps everything else it had');
  // Only `attendees` is patched: a PATCH carrying the whole event body would
  // rewrite fields nobody asked to change.
  assert.deepEqual(Object.keys(sent), ['attendees']);

  assert.equal(out.external_id, 'evt-1');
  assert.equal(out.etag, '"8"');
});

test('the RSVP PATCH carries the etag of the event it just read', () => {
  // The read-modify-write sends the WHOLE attendee array back, so a guest added
  // between the GET and the PATCH is not in the array being sent and would be
  // deleted by it — "array fields, if specified, overwrite the existing arrays",
  // the same sentence the whole operation is built around. If-Match is what
  // turns that window into a visible 412 instead of a silently dropped guest.
  const { handler, calls } = stub([ok(INVITE), ok({ id: 'evt-1' })]);
  handler(rsvp('accept'));
  assert.equal(calls[1].request.headers['If-Match'], '"7"');
});

test('a concurrent edit FAILS the RSVP rather than overwriting the guest list', () => {
  // 412 -> `conflict`, which submit_outcome.rs makes TERMINAL: the command
  // stops, a person requeues it against the current event. The alternative is
  // this RSVP silently removing whoever was invited in the meantime.
  const { handler } = stub([
    ok(INVITE),
    { status: 412, body: { error: { message: 'Precondition Failed' } } },
  ]);
  assert.throws(
    () => handler(rsvp('accept')),
    (e) => e.code === 'conflict'
  );
});

test('accept and tentative map onto Google responseStatus values', () => {
  for (const [action, status] of [['accept', 'accepted'], ['tentative', 'tentative']]) {
    const { handler, calls } = stub([ok(INVITE), ok({ id: 'evt-1' })]);
    handler(rsvp(action));
    assert.equal(JSON.parse(calls[1].request.body).attendees[1].responseStatus, status);
  }
});

test('an RSVP tells the organizer by default, and a mirror write still tells nobody', () => {
  // Telling the organizer IS the RSVP: sendUpdates=none records the response
  // and notifies no one, i.e. a no-op with a green tick on it. That is the
  // opposite of the right default for a MIRROR write, and the two must not
  // share `sendUpdates(mount)`.
  const rsvpCalls = stub([ok(INVITE), ok({ id: 'evt-1' })]);
  rsvpCalls.handler(rsvp('accept'));
  assert.match(rsvpCalls.calls[1].url, /sendUpdates=all/);

  const optedOut = stub([ok(INVITE), ok({ id: 'evt-1' })]);
  optedOut.handler(rsvp('accept', { send_response: false }));
  assert.match(optedOut.calls[1].url, /sendUpdates=none/);

  const mirror = stub([ok({ id: 'evt-2' })]);
  mirror.handler({
    operation: 'update',
    params: { item_id: 'evt-2', payload: { summary: 'Renamed' } },
    credential,
    mount,
  });
  assert.match(mirror.calls[0].url, /sendUpdates=none/, 'a mirror write mails nobody');
});

test('a comment rides on the self row, not as a top-level field', () => {
  const { handler, calls } = stub([ok(INVITE), ok({ id: 'evt-1' })]);
  handler(rsvp('decline', { comment: 'clashes with the release' }));
  const sent = JSON.parse(calls[1].request.body);
  assert.equal(sent.attendees[1].comment, 'clashes with the release');
  assert.equal(sent.comment, undefined);
});

test('no self row FAILS the command rather than reporting one sent', () => {
  // The principal is not an attendee — typically it is the organizer, who has
  // nothing to RSVP to. A command that settles as `sent` without sending is the
  // worst outcome the submit protocol can produce, so this is terminal and named.
  const { handler, calls } = stub([
    ok({ id: 'evt-1', attendees: [{ email: 'ada@example.com', responseStatus: 'accepted' }] }),
  ]);
  assert.throws(
    () => handler(rsvp('accept')),
    (e) => e.code === 'config_error' && /self/.test(e.message)
  );
  assert.equal(calls.length, 1, 'nothing was PATCHed');
});

test('an event that is gone fails the command; it is not the null an update returns', () => {
  const { handler } = stub([{ status: 404, body: { error: { message: 'Not Found' } } }]);
  assert.throws(
    () => handler(rsvp('accept')),
    (e) => e.code === 'config_error' && /no longer exists/.test(e.message)
  );
});

test('an unknown action is refused before any request is made', () => {
  const { handler, calls } = stub([]);
  assert.throws(
    () => handler(rsvp('maybe')),
    (e) => e.code === 'config_error' && /maybe/.test(e.message)
  );
  assert.equal(calls.length, 0);
});

// ---------------------------------------------------------------------------
// browse: calendarList discovery.

test('browse lists calendars so nobody has to type an id, and pages by token', () => {
  const { handler, calls } = stub([
    ok({
      items: [
        { id: 'primary', summary: 'Ada Lovelace', primary: true, accessRole: 'owner' },
        { id: 'team@example.com', summary: 'Team', summaryOverride: 'Our Team', accessRole: 'writer' },
        { id: 'ro@example.com', summary: 'Holidays', accessRole: 'reader' },
        { id: 'gone@example.com', summary: 'Old', deleted: true },
      ],
      nextPageToken: 'p2',
    }),
  ]);
  const out = handler({ operation: 'browse', params: { kind: 'calendar' }, credential, mount });

  assert.match(calls[0].url, /\/users\/me\/calendarList\?/);
  assert.deepEqual(
    out.items.map((i) => [i.id, i.name, i.hint, i.has_children]),
    [
      ['primary', 'Ada Lovelace', 'primary · owner', false],
      // summaryOverride is the name the operator gave it in their own list.
      ['team@example.com', 'Our Team', 'writer', false],
      ['ro@example.com', 'Holidays', 'reader', false],
    ],
    'a deleted calendarList entry is a tombstone: mounting it 404s on the first list'
  );
  assert.equal(out.next_cursor, 'p2');

  const page2 = stub([ok({ items: [] })]);
  page2.handler({ operation: 'browse', params: { cursor: 'p2' }, credential, mount });
  assert.match(page2.calls[0].url, /pageToken=p2/);
});

// ---------------------------------------------------------------------------
// capabilities.

test('the declared surface matches what index.js can actually dispatch', () => {
  // A capability with nothing behind it makes a mount resolve as capable and
  // then throw at drain time, with a command already claimed. So this asserts
  // the DECLARATION against the DISPATCH TABLE, not against a literal.
  const { handler } = stub([]);
  const caps = handler({ operation: 'capabilities', params: {}, credential, mount });
  assert.equal(caps.can_submit, true);
  assert.equal(caps.supports_browse, true);
  // Google has no idempotency header for events.patch; at-most-once rests
  // entirely on the engine's durable claim, so this must stay falsy.
  assert.ok(!caps.supports_idempotency_key);

  for (const op of ['submit', 'browse']) {
    assert.doesNotThrow(
      () => {
        try {
          handler({ operation: op, params: {}, credential, mount });
        } catch (e) {
          // Reaching a coded adapter error proves the case exists; the generic
          // "Unsupported operation" is what an undeclared surface throws.
          if (/Unsupported operation/.test(e.message)) throw e;
        }
      },
      `capabilities declares ${op} but the dispatch table has no case for it`
    );
  }
});
