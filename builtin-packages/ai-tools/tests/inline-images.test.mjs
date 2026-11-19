/**
 * Inline image bytes on a chat message (vision): the content parts the model
 * turn builds, the caps, history stripping and the strip at the end of a run.
 *
 * Run: node --test builtin-packages/ai-tools/tests/inline-images.test.mjs
 */
import test from 'node:test';
import assert from 'node:assert/strict';

import { inlineAttachments } from '../content/functions/lib/raisin/ai/agent-shared/message-images.js';
import {
  planInlineImages, rawInlineOf, hasImageParts, withoutImageParts, INLINE_MAX_B64, INLINE_MAX_IMAGES,
} from '../content/functions/lib/raisin/ai/agent-shared/inline-images.js';
import { cleanAttachments, clientInputOf } from '../content/functions/lib/raisin/ai/agent-shared/client-tools.js';
import { stripInlineBytes } from '../content/functions/lib/raisin/ai/agent-shared/inline-strip.js';

const att = (name) => ({ 'raisin:ref': `/uploads/${name}`, 'raisin:workspace': 'assets', mime_type: 'image/jpeg', name });
const img = (name, data = 'QUJD', media_type = 'image/jpeg') => ({ name, media_type, data });

test('vision on: the latest message becomes text + image parts, the reference line stays', async () => {
  const report = {};
  const out = await inlineAttachments([
    { role: 'user', content: 'old', __attachments: [att('old.jpg')], __inline: [img('old.jpg')] },
    { role: 'assistant', content: 'ok' },
    { role: 'user', content: 'look', __attachments: [att('a.jpg')], __inline: [img('a.jpg', 'QUJD')] },
  ], { vision: true, report });
  assert.equal(out[0].content, 'old', 'older messages never resend bytes');
  assert.ok(Array.isArray(out[2].content));
  assert.deepEqual(out[2].content[1], { type: 'image', data: 'QUJD', media_type: 'image/jpeg' });
  assert.match(out[2].content[0].text, /look[\s\S]*assets:\/uploads\/a\.jpg/);
  assert.deepEqual(report, { sent: 1, dropped: [] });
  assert.equal(out.some((m) => '__inline' in m || '__attachments' in m), false);
});

test('vision off: inline is ignored and dropped, text only', async () => {
  const out = await inlineAttachments([{ role: 'user', content: 'x', __attachments: [att('a.jpg')], __inline: [img('a.jpg')] }]);
  assert.equal(typeof out[0].content, 'string');
  assert.equal(JSON.stringify(out).includes('QUJD'), false);
  const off = await inlineAttachments([{ role: 'user', content: 'x', __attachments: [att('a.jpg')], __inline: [img('a.jpg')] }], { vision: false });
  assert.equal(typeof off[0].content, 'string');
});

test('caps: at most 4 images, oversized and malformed ones dropped with a reason', () => {
  const raw = [
    img('1'), img('2'), img('3'), img('4'), img('5'),
    img('big', 'A'.repeat(INLINE_MAX_B64 + 1)),
    img('bad', 'not base64!!'),
    img('gif-ish', 'QUJD', 'image/tiff'),
    img('empty', ''),
  ];
  const { images, dropped } = planInlineImages(raw);
  assert.equal(images.length, INLINE_MAX_IMAGES);
  const reasons = Object.fromEntries(dropped.map((d) => [d.name, d.reason]));
  assert.deepEqual(reasons, { 5: 'too_many', big: 'too_large', bad: 'not_base64', 'gif-ish': 'unsupported_type', empty: 'empty' });
  assert.equal(planInlineImages([img('edge', 'A'.repeat(INLINE_MAX_B64))]).images.length, 1);
});

test('every image refused: the turn still gets its text', async () => {
  const out = await inlineAttachments([{ role: 'user', content: 'x', __attachments: [att('a.jpg')], __inline: [img('a.jpg', '')] }], { vision: true });
  assert.equal(typeof out[0].content, 'string');
});

test('history side: inline never survives cleanAttachments, so run input and replays carry no bytes', () => {
  const raw = [{ ...att('a.jpg'), inline: { media_type: 'image/jpeg', data: 'QUJD' } }, att('b.jpg')];
  assert.equal(JSON.stringify(cleanAttachments(raw)).includes('QUJD'), false);
  assert.equal(JSON.stringify(clientInputOf({ properties: { metadata: { attachments: raw } } }, {})).includes('QUJD'), false);
  assert.deepEqual(rawInlineOf(raw), [{ name: 'a.jpg', media_type: 'image/jpeg', data: 'QUJD' }]);
  assert.deepEqual(rawInlineOf(null), []);
});

test('a refusing model gets the same request without image parts', () => {
  const msgs = [{ role: 'user', content: [{ type: 'text', text: 'hi' }, { type: 'image', data: 'QUJD', media_type: 'image/png' }] }];
  assert.equal(hasImageParts(msgs), true);
  const plain = withoutImageParts(msgs);
  assert.equal(hasImageParts(plain), false);
  assert.match(plain[0].content, /^hi\n\[The attached image/);
  assert.equal(hasImageParts([{ role: 'user', content: 'text' }]), false);
});

test('stripInlineBytes removes bytes from finished messages, keeps the reference, spares newer ones', async () => {
  const writes = [];
  const withBytes = (data) => ({ role: 'user', metadata: { attachments: [{ ...att('a.jpg'), inline: { media_type: 'image/jpeg', data } }] } });
  globalThis.raisin = {
    sql: { query: async () => [
      { path: '/c/new', created_at: '2026-11-19T10:05:00Z', properties: withBytes('NEW') },
      { path: '/c/old', created_at: '2026-11-19T10:00:00Z', properties: withBytes('OLD') },
      { path: '/c/plain', created_at: '2026-11-19T09:00:00Z', properties: { role: 'user', metadata: { attachments: [att('b.jpg')] } } },
    ] },
    nodes: { updateProperty: async (ws, path, key, value) => { writes.push({ path, key, value }); } },
  };
  const n = await stripInlineBytes('ai', '/c', '2026-11-19T10:01:00Z');
  assert.equal(n, 1);
  assert.equal(writes.length, 1);
  assert.equal(writes[0].path, '/c/old');
  assert.equal(JSON.stringify(writes[0].value).includes('OLD'), false);
  assert.equal(writes[0].value.attachments[0]['raisin:ref'], '/uploads/a.jpg');
});

test('stripInlineBytes never throws', async () => {
  globalThis.raisin = { sql: { query: async () => { throw new Error('down'); } }, nodes: {} };
  assert.equal(await stripInlineBytes('ai', '/c', null), 0);
});

// The live failure (2026-11-19): a stored attachment is a reference envelope, which the server
// normalizes to {raisin:ref, raisin:workspace, raisin:path}, dropping `inline`. The bytes now
// travel in metadata.attachment_images; these pin reading them there and clearing them after.
test('rawInlineOf reads metadata.attachment_images (bytes off the envelope)', () => {
  const envelopes = [{ 'raisin:ref': 'X1', 'raisin:workspace': 'assets', 'raisin:path': '/u/p.png' }];
  const images = [{ path: '/u/p.png', name: 'p.png', media_type: 'image/png', data: 'QUJD' }];
  assert.deepEqual(rawInlineOf(envelopes, images), [{ name: 'p.png', media_type: 'image/png', data: 'QUJD' }]);
  assert.deepEqual(rawInlineOf(envelopes, undefined), []);
});

test('stripInlineBytes removes metadata.attachment_images and keeps the references', async () => {
  const writes = [];
  globalThis.raisin = {
    sql: { query: async () => [
      { path: '/c/m', created_at: '2026-11-19T10:00:00Z', properties: { role: 'user', metadata: {
        attachments: [{ 'raisin:ref': 'X1', 'raisin:workspace': 'assets', 'raisin:path': '/u/p.png' }],
        attachment_images: [{ path: '/u/p.png', media_type: 'image/png', data: 'SECRETBYTES' }],
        client: { client_id: 'c1' },
      } } },
    ] },
    nodes: { updateProperty: async (ws, path, key, value) => { writes.push({ path, key, value }); } },
  };
  assert.equal(await stripInlineBytes('ai', '/c', '2026-11-19T10:01:00Z'), 1);
  assert.equal(JSON.stringify(writes[0].value).includes('SECRETBYTES'), false);
  assert.equal('attachment_images' in writes[0].value, false);
  assert.equal(writes[0].value.attachments[0]['raisin:path'], '/u/p.png');
  assert.equal(writes[0].value.client.client_id, 'c1');
});
