/**
 * Attachments of a user message → asset references on that turn.
 *
 * A message may carry `metadata.attachments` (`{raisin:ref, raisin:workspace,
 * mime_type, name}`, see client-tools.js). On the model request, the LATEST
 * user message that has attachments gets one text line per attachment: name,
 * mime type, workspace and path — enough for the person to say "use that
 * image in the open page" and for the agent to put the path on a media field.
 * Older messages' attachments are not repeated.
 *
 * BARE REFERENCES ONLY, on purpose. The model-turn function runs as SYSTEM,
 * and function code has no way to read a node AS another identity
 * (`raisin.nodes.get` uses the function's own context; there is no
 * run-as-user read). Reading an asset's alt text, extracted text or bytes
 * from here would let a message body name an asset its sender cannot read and
 * get it described back (a confused deputy), and an ownership check on a
 * node's `created_by` is no substitute for a permission check. So nothing is
 * read: no details, no excerpt, no pixels — also with `attachments.vision`,
 * which stays reserved until a permission-checked read exists. The agent can
 * read what the asset carries through its own tools, which run as the user.
 *
 * Only the `assets` workspace is referenced; anything else is dropped (the
 * name alone is kept, so the person's message still reads naturally).
 */

import { cleanAttachments } from './client-tools.js';

export const ATTACHMENT_WORKSPACE = 'assets';

/** The attachments of a chat message node (for the history entry), bounded. */
export function attachmentsOfMessage(props) {
  const meta = props && props.metadata && typeof props.metadata === 'object' ? props.metadata : null;
  return meta ? cleanAttachments(meta.attachments) : [];
}

function line(a) {
  const name = a.name || a['raisin:ref'].split('/').pop();
  if (a['raisin:workspace'] !== ATTACHMENT_WORKSPACE) return `[Attached file: ${name} (${a.mime_type || 'unknown type'})]`;
  return `[Attached asset: ${name} (${a.mime_type || 'unknown type'}) — ${ATTACHMENT_WORKSPACE}:${a['raisin:ref']}]`;
}

/**
 * `messages` (history entries, some carrying `__attachments`) → provider
 * messages. The marker is always removed; the latest user message that has
 * attachments gets its references appended as text. No node is read.
 */
export async function inlineAttachments(messages) {
  const list = Array.isArray(messages) ? messages : [];
  let last = -1;
  for (let i = list.length - 1; i >= 0; i--) {
    if (list[i] && list[i].role === 'user' && Array.isArray(list[i].__attachments) && list[i].__attachments.length) {
      last = i;
      break;
    }
  }
  return list.map((m, i) => {
    const { __attachments, ...entry } = m || {};
    if (i !== last) return entry;
    const text = [typeof entry.content === 'string' ? entry.content : '', ...__attachments.map(line)].filter(Boolean).join('\n');
    return { ...entry, content: text };
  });
}
