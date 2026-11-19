/**
 * Inline image bytes a person's own client attached to a chat message.
 *
 * The BROWSER, signed in as the person, reads the picture it just attached and
 * sends it with the message: `metadata.attachments[i].inline =
 * {media_type, data}` (base64, no `data:` prefix). The engine reads nothing on
 * its own behalf, so nothing here can reach bytes the sender could not read.
 *
 * PURE: no host calls. Everything is bounded and validated here so the model
 * turn can never fail because of an oversized or malformed image; a refused
 * image is dropped with a reason and the turn carries on with the reference.
 */

export const INLINE_MAX_IMAGES = 4;
/** Base64 characters per image (about 1.5 MiB). */
export const INLINE_MAX_B64 = 1572864;
export const INLINE_MEDIA_TYPES = ['image/jpeg', 'image/png', 'image/webp', 'image/gif'];

const B64 = /^[A-Za-z0-9+/]+={0,2}$/;

/**
 * Raw `metadata.attachments` of a message → `[{name, media_type, data}]` for
 * every attachment that carries an `inline` object. Not yet validated (that is
 * {@link planInlineImages}); this only keeps the history entry small and flat.
 */
export function rawInlineOf(attachments) {
  if (!Array.isArray(attachments)) return [];
  const out = [];
  for (const a of attachments) {
    const inline = a && typeof a === 'object' && a.inline && typeof a.inline === 'object' ? a.inline : null;
    if (!inline) continue;
    const ref = typeof a['raisin:path'] === 'string' && a['raisin:path'] ? a['raisin:path'] : a['raisin:ref'];
    const name = typeof a.name === 'string' && a.name ? a.name.slice(0, 200) : String(ref || 'image').split('/').pop();
    out.push({ name, media_type: inline.media_type, data: inline.data });
  }
  return out;
}

/**
 * Decide which inline images go to the model.
 * `{images: [{media_type, data}], dropped: [{name, reason}]}`.
 */
export function planInlineImages(raw) {
  const images = [];
  const dropped = [];
  for (const item of Array.isArray(raw) ? raw : []) {
    const name = (item && item.name) || 'image';
    const mt = item && typeof item.media_type === 'string' ? item.media_type.toLowerCase() : '';
    const data = item && typeof item.data === 'string' ? item.data : '';
    if (!INLINE_MEDIA_TYPES.includes(mt)) { dropped.push({ name, reason: 'unsupported_type' }); continue; }
    if (!data) { dropped.push({ name, reason: 'empty' }); continue; }
    if (data.length > INLINE_MAX_B64) { dropped.push({ name, reason: 'too_large' }); continue; }
    if (!B64.test(data)) { dropped.push({ name, reason: 'not_base64' }); continue; }
    if (images.length >= INLINE_MAX_IMAGES) { dropped.push({ name, reason: 'too_many' }); continue; }
    images.push({ media_type: mt, data });
  }
  return { images, dropped };
}

/** True when any message carries an image content part. */
export function hasImageParts(messages) {
  return (Array.isArray(messages) ? messages : []).some(
    (m) => m && Array.isArray(m.content) && m.content.some((p) => p && (p.type === 'image' || p.type === 'image_url')),
  );
}

/** The same messages with every image part replaced by a one-line note. */
export function withoutImageParts(messages) {
  return (Array.isArray(messages) ? messages : []).map((m) => {
    if (!m || !Array.isArray(m.content)) return m;
    const text = m.content.filter((p) => p && p.type === 'text').map((p) => p.text).filter(Boolean).join('\n');
    const note = '[The attached image(s) could not be shown to this model; only their references are available.]';
    return { ...m, content: [text, note].filter(Boolean).join('\n') };
  });
}
