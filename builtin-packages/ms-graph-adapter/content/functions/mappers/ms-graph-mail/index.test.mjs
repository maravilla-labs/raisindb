// Run with: node --test builtin-packages/ms-graph-adapter/content/functions/mappers/ms-graph-mail/
//
// index.js is loaded the way the engine loads it — a bare script whose entry
// point is the global `handler` — so there is nothing to import.

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8');
const handler = new Function(`${src}\nreturn handler;`)();

const mount = { mount_id: 'm1', mount_path: '/mail', remote_root: 'inbox', sync_config: {} };

function map(metadata, item = {}) {
  return handler({
    operation: 'to_node',
    mount,
    external_item: {
      external_id: 'AAMkAGI2',
      name: 'AAMkAGI2',
      size_bytes: 8123,
      metadata,
      ...item,
    },
  });
}

test('the body split replaces the ambiguous body + body_type pair', () => {
  const html = map({ subject: 'Hi', body_html: '<p>hi</p>' });
  assert.equal(html.properties.body_html, '<p>hi</p>');
  assert.equal(html.properties.body_text, undefined);
  // A single column whose meaning depended on a sibling column could not be
  // searched, rendered or replied to without branching. Neither survives.
  assert.equal(html.properties.body, undefined);
  assert.equal(html.properties.body_type, undefined);

  const text = map({ subject: 'Hi', body_text: 'hi' });
  assert.equal(text.properties.body_text, 'hi');
  assert.equal(text.properties.body_html, undefined);
});

test('an absent body leaves BOTH columns unset, never blanked', () => {
  // Writing "" would blank a previously synced body and change the node on
  // every run, defeating the etag skip-write that stops a re-sync re-firing
  // every downstream trigger.
  const out = map({ subject: 'Link-only' });
  assert.equal('body_html' in out.properties, false);
  assert.equal('body_text' in out.properties, false);
});

test('the draft/sent distinction travels as three separate columns', () => {
  const received = map({
    date: '2026-08-01T10:00:00Z',
    received_at: '2026-08-01T10:00:00Z',
    sent_at: '2026-08-01T09:59:00Z',
  });
  assert.equal(received.properties.date, '2026-08-01T10:00:00Z');
  assert.equal(received.properties.received_at, '2026-08-01T10:00:00Z');
  assert.equal(received.properties.is_draft, false);

  // An outgoing copy: no received_at. That absence IS how it is recognised,
  // without branching on a folder name.
  const sent = map({ date: '2026-08-01T09:59:00Z', sent_at: '2026-08-01T09:59:00Z' });
  assert.equal(sent.properties.received_at, null);
  assert.equal(sent.properties.sent_at, '2026-08-01T09:59:00Z');

  assert.equal(map({ is_draft: true }).properties.is_draft, true);
});

test('bcc, reply_to, labels and size reach the node', () => {
  const out = map({
    bcc: 'Hidden <h@x.test>',
    reply_to: 'Desk <desk@x.test>',
    labels: ['Red category'],
  });
  assert.equal(out.properties.bcc, 'Hidden <h@x.test>');
  assert.equal(out.properties.reply_to, 'Desk <desk@x.test>');
  // Outlook categories and Gmail labels share one column on purpose: a consumer
  // must not branch on provider to read a label.
  assert.deepEqual(out.properties.labels, ['Red category']);
  assert.equal(out.properties.size, 8123);
});

test('attachments become raisin:Asset children, metadata only', () => {
  const out = map({
    has_attachments: true,
    attachments: [
      { external_id: 'a1', name: 'quote.pdf', mime_type: 'application/pdf', size: 2048 },
      {
        external_id: 'a2',
        name: 'logo.png',
        mime_type: 'image/png',
        size: 512,
        inline: true,
        content_id: '<logo>',
      },
    ],
  });

  assert.equal(out.children.length, 2);
  const [quote, logo] = out.children;
  assert.equal(quote.node_type, 'raisin:Asset');
  // The attachment ID, not the filename. `name` is what the child's node PATH is
  // built from, and a filename is not unique: two `scan.pdf` attachments on one
  // message resolved onto a single path and the engine silently kept the last.
  assert.equal(quote.name, 'a1');
  assert.equal(quote.properties.title, 'quote.pdf', 'the filename stays displayable');
  assert.equal(quote.external_id, 'a1');
  assert.equal(quote.properties.file_size, 2048);
  // NO `file`. Its absence is what the engine's on-demand get_content keys off;
  // a placeholder Resource pointing at nothing reads as "the bytes are there".
  assert.equal('file' in quote.properties, false);

  assert.equal(logo.properties.inline, true);
  // Angle brackets stripped: body_html references it as `cid:logo`.
  assert.equal(logo.properties.content_id, 'logo');
});

test('no attachment list means "not synced", and emits no children key at all', () => {
  // An empty array would tell the engine every attachment node it holds is gone
  // upstream, and a mount without `include_attachments` would delete them all.
  const flagged = map({ has_attachments: true });
  assert.equal('children' in flagged, false);
  assert.equal(flagged.properties.has_attachments, true);

  assert.equal('children' in map({ attachments: [] }), false);
});

test('the message itself is still raisin:Mail, keyed on the Graph id', () => {
  const out = map({ subject: 'Hi' });
  assert.equal(out.node_type, 'raisin:Mail');
  // id, not subject — path stability is owned by the adapter's external_id.
  assert.equal(out.name, 'AAMkAGI2');
  assert.equal(out.properties.subject, 'Hi');
  assert.equal(out.properties.folder, 'inbox');
});

test('to_external still pushes only the read flag', () => {
  assert.deepEqual(handler({ operation: 'mapper_capabilities', mount }), {
    to_external: true,
  });
  const out = handler({
    operation: 'to_external',
    mount,
    node: { properties: { unread: false, subject: 'do not send me', __external_id: 'x' } },
  });
  assert.deepEqual(out, { payload: { isRead: true }, external_id: 'x' });
});

test('unread alone in fields produces a write — fields is the diverged-only subset', () => {
  // The engine sends ONLY the fields whose value moved off the __pushed_state
  // baseline. A mapper that needs company for a field returns null instead of
  // a payload, the push is Skipped, and the node is re-nominated forever with
  // no request ever going out (the Hue colour-pair bug).
  const out = handler({
    operation: 'to_external',
    mount,
    fields: ['unread'],
    node: { properties: { unread: true, __external_id: 'x' } },
  });
  assert.deepEqual(out, { payload: { isRead: false }, external_id: 'x' });
});

test('is_read alone in fields produces a write, with no inversion', () => {
  // The Graph-truth alias: is_read carries isRead's own polarity, unlike
  // `unread` which inverts.
  const out = handler({
    operation: 'to_external',
    mount,
    fields: ['is_read'],
    node: { properties: { is_read: true, __external_id: 'x' } },
  });
  assert.deepEqual(out, { payload: { isRead: true }, external_id: 'x' });
});

test('when both spellings diverged, the canonical unread column decides', () => {
  // They can only disagree when a caller wrote one spelling over stale sync
  // state in the other; one deterministic winner, never two payload writes.
  const out = handler({
    operation: 'to_external',
    mount,
    fields: ['is_read', 'unread'],
    node: { properties: { unread: true, is_read: true } },
  });
  assert.deepEqual(out.payload, { isRead: false });
});

test('a node that never carried the flag is not pushed as "read"', () => {
  // Absent is not false: null, not an empty PATCH that bumps the change key.
  assert.equal(
    handler({ operation: 'to_external', mount, fields: ['is_read'], node: { properties: {} } }),
    null
  );
  assert.equal(
    handler({ operation: 'to_external', mount, fields: ['unread'], node: { properties: {} } }),
    null
  );
});

test('to_node emits both spellings of the read flag, mutually inverse', () => {
  // Both must live in the __pushed_state baseline, or an edit to the missing
  // spelling never diverges and is never nominated — the "toggle is silently
  // inert" symptom.
  const read = map({ unread: false });
  assert.equal(read.properties.unread, false);
  assert.equal(read.properties.is_read, true);

  const fresh = map({ unread: true });
  assert.equal(fresh.properties.unread, true);
  assert.equal(fresh.properties.is_read, false);
});

// ---- importance is writable -----------------------------------------------

test('importance pushes as its own PATCH', () => {
  const out = handler({
    operation: 'to_external',
    node: { properties: { importance: 'high', __external_id: 'M1' } },
    fields: ['importance'],
  })
  assert.deepEqual(out.payload, { importance: 'high' })
  assert.equal(out.external_id, 'M1')
})

test('a diverged importance alone is a complete request', () => {
  // `fields` is the DIVERGED subset, so importance must stand on its own —
  // gating it on the read flag arriving too is how a one-field divergence
  // becomes a null return and a node re-nominated forever.
  const out = handler({
    operation: 'to_external',
    node: { properties: { unread: true, importance: 'low' } },
    fields: ['importance'],
  })
  assert.deepEqual(out.payload, { importance: 'low' })
})

test('importance is normalised, and a value Graph does not know is dropped', () => {
  const cased = handler({
    operation: 'to_external',
    node: { properties: { importance: 'High' } },
    fields: ['importance'],
  })
  assert.deepEqual(cased.payload, { importance: 'high' })

  // Dropped rather than sent: Graph answers a bad value with a 400, which the
  // engine reads as a config error and stands the mount off — too heavy a
  // price for one mistyped word on one message.
  const nonsense = handler({
    operation: 'to_external',
    node: { properties: { importance: 'urgent!' } },
    fields: ['importance'],
  })
  assert.equal(nonsense, null)
})

test('the read flag and importance travel together when both diverged', () => {
  const out = handler({
    operation: 'to_external',
    node: { properties: { unread: false, importance: 'normal' } },
    fields: ['unread', 'importance'],
  })
  assert.deepEqual(out.payload, { isRead: true, importance: 'normal' })
})
