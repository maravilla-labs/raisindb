/**
 * Take the inline image bytes back OFF the conversation once a run is over.
 *
 * The person's client writes `inline` onto the message it sends; the run that
 * answers it needs the bytes on every model turn (each turn rebuilds its
 * context from the nodes), so they cannot go earlier. At the end of the run the
 * reference stays and the bytes go, so a conversation does not grow by a
 * megapixel per attachment and a later turn can never replay them.
 *
 * Best effort: a failed strip is logged and never fails the run.
 */

import { log } from './logger.js';

/**
 * Strip `inline` from the user messages of `chatPath` created at or before
 * `before` (an ISO time: the run's last assistant turn, so a message that
 * arrived for the NEXT run is left alone). Returns how many were rewritten.
 */
export async function stripInlineBytes(workspace, chatPath, before) {
  let rewritten = 0;
  try {
    const rows = await raisin.sql.query(`
      SELECT path, properties, created_at FROM '${workspace}'
      WHERE CHILD_OF($1) AND node_type = 'raisin:Message'
      ORDER BY created_at DESC LIMIT 12
    `, [chatPath]);
    const cutoff = before ? new Date(before).getTime() : Infinity;
    for (const row of Array.isArray(rows) ? rows : []) {
      const props = row.properties || {};
      const meta = props.metadata && typeof props.metadata === 'object' ? props.metadata : null;
      if (props.role !== 'user' || !meta) continue;
      if (new Date(row.created_at).getTime() > cutoff) continue;
      const list = Array.isArray(meta.attachments) ? meta.attachments : [];
      const onEnvelope = list.some((a) => a && typeof a === 'object' && 'inline' in a);
      const separate = 'attachment_images' in meta;
      if (!onEnvelope && !separate) continue;
      const { attachment_images, ...kept } = meta;
      const attachments = list.map((a) => {
        if (!a || typeof a !== 'object' || !('inline' in a)) return a;
        const { inline, ...rest } = a;
        return rest;
      });
      await raisin.nodes.updateProperty(workspace, row.path, 'metadata', list.length ? { ...kept, attachments } : kept);
      rewritten += 1;
    }
  } catch (err) {
    log.warn('inline-strip', 'Could not strip inline image bytes', { error: String((err && err.message) || err) });
  }
  return rewritten;
}
