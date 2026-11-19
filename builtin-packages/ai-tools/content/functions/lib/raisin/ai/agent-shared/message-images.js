/**
 * Image attachments of a user message → content parts on that turn.
 *
 * A message may carry `metadata.attachments` (`{raisin:ref, raisin:workspace,
 * mime_type, name}`, see client-tools.js). On the model request, the LATEST
 * user message that has attachments becomes `[text, image parts…]`; non-image
 * files stay as text references (name and path), which is all a model can use
 * without a document tool. Older messages' attachments are not re-sent every
 * turn (cost): the model has already seen them in its history.
 *
 * Reading the bytes happens in the model-turn function, which runs as SYSTEM.
 * So an attachment is read only when the person it is for CREATED the asset
 * (`created_by` matches the run owner or the message sender): a reference in
 * a message body must never be a way to read an asset the sender cannot.
 *
 * Caps: 4 images per turn, 5 MB per image; anything over is listed as text.
 */

import { log } from './logger.js';
import { cleanAttachments } from './client-tools.js';

export const MAX_IMAGES_PER_TURN = 4;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;

/** The attachments of a chat message node (for the history entry), bounded. */
export function attachmentsOfMessage(props) {
  const meta = props && props.metadata && typeof props.metadata === 'object' ? props.metadata : null;
  return meta ? cleanAttachments(meta.attachments) : [];
}

function ownedBy(node, owners) {
  const by = [node && node.created_by, node && node.properties && node.properties.created_by]
    .filter((v) => typeof v === 'string' && v);
  return by.length > 0 && by.some((v) => owners.includes(v));
}

function label(a) {
  return `${a.name || a['raisin:ref'].split('/').pop()} (${a.mime_type || 'unknown type'}) — ${a['raisin:workspace']}:${a['raisin:ref']}`;
}

/** One image attachment → an image part, or a text note saying why not. */
async function imagePart(a, owners) {
  try {
    const node = await raisin.nodes.get(a['raisin:workspace'], a['raisin:ref']);
    if (!node) return { type: 'text', text: `[Attached image not found: ${label(a)}]` };
    if (!ownedBy(node, owners)) {
      return { type: 'text', text: `[Attached image not readable for this conversation: ${label(a)}]` };
    }
    const resource = typeof node.getResource === 'function' ? node.getResource('file') : null;
    if (!resource) return { type: 'text', text: `[Attached image has no file: ${label(a)}]` };
    const size = Number(resource.size);
    if (Number.isFinite(size) && size > MAX_IMAGE_BYTES) {
      return { type: 'text', text: `[Attached image too large to show (${size} bytes, limit ${MAX_IMAGE_BYTES}): ${label(a)}]` };
    }
    const data = await resource.getBinary();
    if (typeof data !== 'string' || !data) return { type: 'text', text: `[Attached image unreadable: ${label(a)}]` };
    // Base64 is 4/3 of the bytes; hold the cap when the size was unknown.
    if (data.length > Math.ceil((MAX_IMAGE_BYTES * 4) / 3) + 4) {
      return { type: 'text', text: `[Attached image too large to show: ${label(a)}]` };
    }
    return { type: 'image', data, media_type: resource.mimeType || a.mime_type };
  } catch (err) {
    log.warn('message-images', 'Attachment unreadable', { ref: a['raisin:ref'], error: String(err && err.message) });
    return { type: 'text', text: `[Attached image unreadable: ${label(a)}]` };
  }
}

/**
 * `messages` (history entries, some carrying `__attachments`) → provider
 * messages. The marker is always removed; the latest user message that has
 * attachments gets its content parts.
 */
export async function inlineAttachments(messages, { owners = [] } = {}) {
  const list = Array.isArray(messages) ? messages : [];
  let last = -1;
  for (let i = list.length - 1; i >= 0; i--) {
    if (list[i] && list[i].role === 'user' && Array.isArray(list[i].__attachments) && list[i].__attachments.length) {
      last = i;
      break;
    }
  }
  const out = [];
  for (let i = 0; i < list.length; i++) {
    const { __attachments, ...entry } = list[i] || {};
    if (i !== last) {
      out.push(entry);
      continue;
    }
    const parts = [];
    const notes = [];
    let images = 0;
    for (const a of __attachments) {
      if (/^image\//i.test(a.mime_type) && images < MAX_IMAGES_PER_TURN) {
        images += 1;
        parts.push(await imagePart(a, owners));
      } else if (/^image\//i.test(a.mime_type)) {
        notes.push(`[Attached image not shown (more than ${MAX_IMAGES_PER_TURN} this turn): ${label(a)}]`);
      } else {
        notes.push(`[Attached file: ${label(a)}]`);
      }
    }
    const text = [typeof entry.content === 'string' ? entry.content : '', ...notes].filter(Boolean).join('\n');
    out.push({ ...entry, content: [{ type: 'text', text: text || '(attached)' }, ...parts] });
  }
  return out;
}
