/**
 * Attachments of a user message → asset references (and, with vision, image parts) on that turn.
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

export const MAX_EXCERPT_CHARS = 1500;

/** What the asset already carries (pipeline output), as short text; '' when nothing. */
function assetFacts(node) {
  const p = (node && node.properties) || {};
  const out = [];
  const add = (k, v) => {
    if (typeof v === 'string' && v.trim()) out.push(`  ${k}: ${v.trim().slice(0, 500)}`);
  };
  add('title', p.title);
  add('alt text', p.alt_text);
  add('description', p.description || p.caption);
  if (Array.isArray(p.keywords) && p.keywords.length) add('keywords', p.keywords.join(', '));
  const text = typeof p.__extracted_text === 'string' ? p.__extracted_text.trim() : '';
  if (text) out.push(`  extracted text (excerpt): ${text.slice(0, MAX_EXCERPT_CHARS)}${text.length > MAX_EXCERPT_CHARS ? ' …' : ''}`);
  return out.join('\n');
}

/** An image attachment of a vision agent → an image part, or a note saying why not. */
async function imagePart(a, node) {
  try {
    const resource = typeof node.getResource === 'function' ? node.getResource('file') : null;
    if (!resource) return { type: 'text', text: `[Attached image has no file to show: ${label(a)}]` };
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

async function readNode(a) {
  try {
    return await raisin.nodes.get(a['raisin:workspace'], a['raisin:ref']);
  } catch (_) {
    return null;
  }
}

/**
 * `messages` (history entries, some carrying `__attachments`) → provider
 * messages. The marker is always removed; the latest user message that has
 * attachments gets a text reference per attachment (path, workspace, name,
 * type, alt text / description / extracted-text excerpt) and, only with
 * `vision` (agent `attachments.vision: true`), image parts as well.
 */
export async function inlineAttachments(messages, { owners = [], vision = false } = {}) {
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
    const lines = [];
    let images = 0;
    for (const a of __attachments) {
      // The asset is the attachment: a reference the agent can use in the open
      // document, with what the pipeline already knows about it. The details
      // (and the pixels) are read only for assets the sender created.
      const node = await readNode(a);
      const mine = !!node && ownedBy(node, owners);
      const facts = mine ? assetFacts(node) : '';
      lines.push(`[Attached asset: ${label(a)}${facts ? `\n${facts}` : ''}]`);
      if (vision && mine && /^image\//i.test(a.mime_type)) {
        if (images < MAX_IMAGES_PER_TURN) {
          images += 1;
          parts.push(await imagePart(a, node));
        } else {
          lines.push(`[Image not shown (more than ${MAX_IMAGES_PER_TURN} this turn): ${a.name}]`);
        }
      }
    }
    const text = [typeof entry.content === 'string' ? entry.content : '', ...lines].filter(Boolean).join('\n');
    out.push({ ...entry, content: parts.length ? [{ type: 'text', text: text || '(attached)' }, ...parts] : (text || '(attached)') });
  }
  return out;
}
