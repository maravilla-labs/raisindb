/**
 * Microsoft 365 mail mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
 *
 * Messages map to a message-ish raisin:Node (title=subject, from, to, date,
 * snippet, unread), not raisin:Asset: a mail message is metadata + a body we do
 * not inline during ordinary sync. The body is available on demand via the
 * adapter's get_content.
 *
 * `name` stays the Graph item id (external_item.name) so distinct messages never
 * collide on a path; the human-readable subject lives in the `title` property.
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  var meta = item.metadata || {};
  var subject = meta.subject || "(no subject)";

  return {
    node_type: "raisin:Node",
    // id, not subject — path stability is owned by the adapter's external_id.
    name: item.name,
    properties: {
      title: subject,
      from: meta.from || null,
      to: meta.to || null,
      cc: meta.cc || null,
      date: meta.date || item.modified_at || null,
      snippet: meta.snippet || null,
      unread: meta.unread === true,
      has_attachments: meta.has_attachments === true,
      conversation_id: meta.conversation_id || null,
      web_url: item.web_url || meta.web_url || null,
      provider: "ms-graph",
      provider_kind: "mail",
      mime_type: "message/rfc822",
      // Provider-specific passthrough is preserved verbatim.
      provider_metadata: meta,
    },
  };
}
