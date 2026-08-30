// Run with: node --test builtin-packages/imap-adapter/content/functions/mappers/imap-default/
//
// index.js is loaded the way the engine loads it — a bare script whose entry
// point is the global `handler` — so there is nothing to import.

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./index.js', import.meta.url), 'utf8');
const handler = new Function(`${src}\nreturn handler;`)();

const mount = { mount_id: 'm1', mount_path: '/mail', remote_root: 'INBOX', sync_config: {} };

function map(metadata, item = {}) {
  return handler({
    mount,
    external_item: {
      external_id: '42',
      name: 'Hello',
      mime_type: 'message/rfc822',
      size_bytes: 4096,
      metadata,
      ...item,
    },
  });
}

test('a message is raisin:Mail, not raisin:Node', () => {
  // The divergence this mapper shipped with. `raisin:Mail` declares the
  // Fulltext and Property indexing that makes mail queryable, so every
  // IMAP-synced message used to be a raisin:Node carrying mail-shaped
  // properties that no mail query could find — a GROUP BY conversation_id
  // silently covered only the Graph half of a mailbox.
  const out = map({ subject: 'Hello' });
  assert.equal(out.node_type, 'raisin:Mail');
  assert.equal(out.properties.provider, 'imap');
  assert.equal(out.properties.folder, 'INBOX');
});

test('a mailbox is still raisin:Folder', () => {
  const out = handler({
    mount,
    external_item: { external_id: 'INBOX/Sent', name: 'Sent', is_folder: true },
  });
  assert.equal(out.node_type, 'raisin:Folder');
});

test('property names follow the global nodetype, not the IMAP wire shape', () => {
  const out = map({
    subject: 'Hello',
    from: 'Ada <ada@example.org>',
    to: 'bob@example.org',
    date: '2026-08-01T10:00:00Z',
  });
  // `title` was the old name and is not a raisin:Mail column.
  assert.equal(out.properties.title, undefined);
  assert.equal(out.properties.subject, 'Hello');
  assert.equal(out.properties.from, 'Ada <ada@example.org>');
  // `from` carries whatever display name the sender currently uses, which makes
  // it useless as a GROUP BY key — hence the separate bare address.
  assert.equal(out.properties.from_address, 'ada@example.org');
  assert.equal(out.properties.received_at, '2026-08-01T10:00:00Z');
  assert.equal(out.properties.size, 4096);
});

test('a bare address with no display name passes through unchanged', () => {
  assert.equal(map({ from: 'ada@example.org' }).properties.from_address, 'ada@example.org');
  assert.equal(map({}).properties.from_address, null);
});

test('flags are normalized, and \\Draft becomes is_draft', () => {
  const out = map({ flags: ['\\Seen', '\\Draft', '\\Flagged'] });
  // Backslashes stripped and lowercased so the column means the same thing
  // whichever server produced it; the raw form stays in provider_metadata.
  assert.deepEqual(out.properties.flags, ['seen', 'draft', 'flagged']);
  assert.equal(out.properties.is_draft, true);
  assert.deepEqual(out.properties.provider_metadata.flags, ['\\Seen', '\\Draft', '\\Flagged']);

  assert.equal(map({ flags: ['\\Seen'] }).properties.is_draft, false);
});

test('threading headers are read case-insensitively and References is an Array', () => {
  const out = map({
    headers: {
      'In-Reply-To': '<a@x.test>',
      references: '<a@x.test> <b@x.test>',
      CC: 'carol@x.test',
      'Reply-To': 'desk@x.test',
    },
  });
  assert.equal(out.properties.in_reply_to, '<a@x.test>');
  // Not the raw space-joined header: a consumer walking an ancestry should not
  // have to re-parse a header it was handed.
  assert.deepEqual(out.properties.references, ['<a@x.test>', '<b@x.test>']);
  assert.equal(out.properties.cc, 'carol@x.test');
  assert.equal(out.properties.reply_to, 'desk@x.test');
});

test('no headers at all is not an error', () => {
  const out = map({ subject: 'Hi' });
  assert.equal(out.properties.in_reply_to, null);
  assert.equal(out.properties.references, null);
  assert.equal(out.properties.cc, null);
});

test('attachments become raisin:Asset children keyed on the MIME part', () => {
  const out = map({
    attachments: [
      { part: '2', name: 'quote.pdf', mime_type: 'application/pdf', size: 2048 },
      { part: '3', name: 'logo.png', disposition: 'inline', content_id: '<logo>' },
    ],
  });
  assert.equal(out.properties.has_attachments, true);
  assert.equal(out.children.length, 2);
  assert.equal(out.children[0].node_type, 'raisin:Asset');
  // A part number is "2" on nearly every message that has an attachment — the
  // engine namespaces it under the message id so two messages cannot collide.
  assert.equal(out.children[0].external_id, '2');
  assert.equal('file' in out.children[0].properties, false);
  assert.equal(out.children[1].properties.inline, true);
  assert.equal(out.children[1].properties.content_id, 'logo');
});

test('no attachment list emits no children key and no false flag', () => {
  const out = map({ subject: 'Hi' });
  assert.equal('children' in out, false);
  assert.equal(out.properties.has_attachments, false);
});

test('an item with no external_id is skipped', () => {
  assert.equal(handler({ mount, external_item: { name: 'x' } }), null);
  assert.equal(handler({ mount }), null);
});

test('every time column is ISO 8601, and the RAW header is kept out of them', () => {
  // The adapter normalises the Date header into `meta.date` and preserves the
  // sender's original string in `headers.date`. `sent_at` used to be read from
  // that raw header, which put "Wed, 2 Sep ..." into a column the nodetype
  // declares as ISO 8601 and indexes as a Property — a second wire shape for
  // one field, sorting by weekday name.
  const out = map({
    subject: 'Hello',
    date: '2026-09-02T07:00:00Z',
    headers: { date: 'Wed, 2 Sep 2026 09:00:00 +0200', subject: 'Hello' },
  });
  for (const column of ['date', 'received_at', 'sent_at']) {
    assert.equal(out.properties[column], '2026-09-02T07:00:00Z', column);
  }
  // Nothing is lost: the raw header is still one hop away.
  assert.equal(
    out.properties.provider_metadata.headers.date,
    'Wed, 2 Sep 2026 09:00:00 +0200',
  );
});

test('a message the adapter could not date has no invented time', () => {
  // The adapter emits an empty date when the header will not parse; the mapper
  // must write an ABSENT column, never "now" and never the unparsable string.
  const out = map({ subject: 'Hello', date: '', headers: { date: 'not a date' } });
  assert.equal(out.properties.date, null);
  assert.equal(out.properties.received_at, null);
  assert.equal(out.properties.sent_at, null);
});
