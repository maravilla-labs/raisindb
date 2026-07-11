/**
 * Default IMAP mapping function.
 *
 * Called once per external item by the sync engine (adapter contract §6). Pure
 * and fast: it must NOT call raisin.functions.call or perform any I/O — it runs
 * in the sync hot loop. Returning null skips the item.
 *
 *   input  = { external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
 *   return = { node_type, name?, properties } | null
 *
 * Mapping:
 *   - mailboxes -> raisin:Folder
 *   - messages  -> raisin:Node (a message-ish node: subject, from, to, date,
 *                  snippet, message_id, unread flag)
 *
 * Messages map to raisin:Node, not raisin:Asset: a mailbox message is metadata +
 * a body we do not inline during ordinary sync, and raisin:Asset requires a binary
 * file Resource. The body is available on demand via the adapter's get_content.
 *
 * `metadata.from` / `metadata.to` are already-formatted address strings from the
 * native raisin.imap binding (e.g. "Ada <ada@example.org>"), not arrays.
 *
 * The engine stamps the reserved __virtual/__mount_id/__external_id/__etag/
 * __synced_at properties on top of whatever is returned, so they are not set here.
 */

function handler(input) {
  var item = input.external_item;
  if (!item || !item.external_id) return null;

  if (item.is_folder) {
    return {
      node_type: "raisin:Folder",
      name: item.name,
      properties: {
        title: item.name,
        icon: "folder",
      },
    };
  }

  var meta = item.metadata || {};
  var subject = meta.subject || item.name || "(no subject)";

  var properties = {
    title: subject,
    from: meta.from || null,
    to: meta.to || null,
    date: meta.date || item.modified_at || null,
    snippet: meta.snippet || null,
    message_id: meta.message_id || null,
    thread_id: meta.thread_id || null,
    unread: meta.unread === true,
    flags: meta.flags || [],
    provider: "imap",
    mime_type: item.mime_type || "message/rfc822",
    size: item.size_bytes != null ? item.size_bytes : null,
    // Provider-specific passthrough is preserved verbatim.
    provider_metadata: meta,
  };

  return {
    node_type: "raisin:Node",
    name: subject,
    properties: properties,
  };
}
